//! Dealing with individual IRC virtual users.

use irc::client::PackedIrcClient;
use futures::sync::mpsc::{UnboundedSender, UnboundedReceiver, self};
use crate::comm::{ModemCommand, ContactManagerCommand, WhatsappCommand, InitParameters};
use huawei_modem::pdu::{PduAddress, DeliverPdu};
use crate::store::Store;
use failure::Error;
use futures::{self, Future, Async, Poll, Stream};
use futures::future::Either;
use std::default::Default;
use irc::client::{IrcClient, ClientStream, Client};
use irc::client::ext::ClientExt;
use irc::proto::command::Command;
use irc::proto::response::Response;
use irc::proto::message::Message;
use crate::models::Recipient;
use crate::config::IrcClientConfig;
use irc::client::data::config::Config as IrcConfig;
use crate::util::{self, Result};
use crate::sender_common::Sender;

pub struct ContactManager {
    irc: PackedIrcClient,
    irc_stream: ClientStream,
    admin: String,
    nick: String,
    addr: PduAddress,
    store: Store,
    id: bool,
    wa_mode: bool,
    admin_is_online: bool,
    connected: bool,
    presence: Option<String>,
    channels: Vec<String>,
    webirc_password: Option<String>,
    wa_tx: UnboundedSender<WhatsappCommand>,
    modem_tx: UnboundedSender<ModemCommand>,
    pub tx: UnboundedSender<ContactManagerCommand>,
    rx: UnboundedReceiver<ContactManagerCommand>
}
impl Future for ContactManager {
    type Item = ();
    type Error = Error;

    fn poll(&mut self) -> Poll<(), Error> {
        if !self.id {
            if let Some(ref pw) = self.webirc_password {
                let vhost = format!("{}.sms-irc.theta.eu.org", util::string_to_irc_nick(&self.addr.to_string())); 
                self.irc.0.send(Command::Raw("WEBIRC".into(), vec![pw.to_string(), "sms-irc".into(), vhost, "127.0.0.1".into()], None))?;
            }
            self.irc.0.identify()?;
            self.id = true;
        }
        while let Async::Ready(_) = self.irc.1.poll()? {}
        while let Async::Ready(res) = self.irc_stream.poll()? {
            let msg = res.ok_or(format_err!("irc_stream stopped"))?;
            self.handle_irc_message(msg)?;
        }
        while let Async::Ready(cmc) = self.rx.poll().unwrap() {
            let cmc = cmc.ok_or(format_err!("contactmanager rx died"))?;
            self.handle_int_rx(cmc)?;
        }
        while let Async::Ready(_) = self.irc.1.poll()? {}
        Ok(Async::NotReady)
    }
}
impl Sender for ContactManager {
    fn report_error(&mut self, _: &str, err: String) -> Result<()> {
        self.irc.0.send_notice(&self.admin, &err)?;
        Ok(())
    }
    fn store(&mut self) -> &mut Store {
        &mut self.store
    }
    fn private_target(&mut self) -> String {
        self.admin.clone()
    }
    fn send_irc_message(&mut self, _: &str, to: &str, msg: &str) -> Result<()> {
        self.irc.0.send_privmsg(to, msg)?;
        Ok(())
    }
}
impl ContactManager {
    pub fn add_command(&self, cmd: ContactManagerCommand) {
        self.tx.unbounded_send(cmd)
            .unwrap()
    }
    pub fn nick(&self) -> &str {
        &self.nick
    }
    fn process_groups(&mut self) -> Result<()> {
        if !self.connected {
            debug!("Not processing group changes yet; not connected.");
            return Ok(());
        }
        debug!("Processing group changes");
        let mut chans = vec![];
        for grp in self.store.get_groups_for_recipient(&self.addr)? {
            debug!("Joining {}", grp.channel);
            self.irc.0.send_join(&grp.channel)?;
            chans.push(grp.channel);
        }
        for ch in ::std::mem::replace(&mut self.channels, chans) {
            if !self.channels.contains(&ch) {
                debug!("Parting {}", ch);
                self.irc.0.send_part(&ch)?;
            }
        }
        Ok(())
    }
    
    fn process_messages(&mut self) -> Result<()> {
        use std::convert::TryFrom;

        if !self.connected {
            debug!("Not processing messages yet; not connected");
            return Ok(());
        }
        if !self.admin_is_online {
            debug!("Not processing messages; admin offline");
            return Ok(());
        }

        let msgs = self.store.get_messages_for_recipient(&self.addr)?;
        for msg in msgs {
            debug!("Processing message #{}", msg.id);
            if msg.pdu.is_some() {
                let pdu = DeliverPdu::try_from(msg.pdu.as_ref().unwrap() as &[u8])?;
                if self.wa_mode {
                    self.wa_mode = false;
                    self.store.update_recipient_wa(&self.addr, self.wa_mode)?;
                    self.irc.0.send_notice(&self.admin, "Notice: SMS mode automatically enabled.")?;
                }
                self.process_msg_pdu("", msg, pdu)?;
            }
            else {
                if !self.wa_mode {
                    self.wa_mode = true;
                    self.store.update_recipient_wa(&self.addr, self.wa_mode)?;
                    self.irc.0.send_notice(&self.admin, "Notice: WhatsApp mode automatically enabled.")?;
                }
                self.process_msg_plain("", msg)?;
            }
        }
        Ok(())
    }
    fn update_away(&mut self) -> Result<()> {
        if !self.connected {
            debug!("Not updating presence yet; not connected");
            return Ok(());
        }
        debug!("Setting away state to {:?}", self.presence);
        self.irc.0.send(Command::AWAY(self.presence.clone()))?;
        Ok(())
    }
    fn handle_int_rx(&mut self, cmc: ContactManagerCommand) -> Result<()> {
        use self::ContactManagerCommand::*;
        match cmc {
            ProcessMessages => self.process_messages()?,
            ProcessGroups => self.process_groups()?,
            UpdateAway(msg) => {
                self.presence = msg;
                self.update_away()?;
            },
            ChangeNick(nick, src) => {
                self.change_nick(nick, src)?;
            },
            SetWhatsapp(wam) => {
                self.wa_mode = wam;
                self.store.update_recipient_wa(&self.addr, self.wa_mode)?;
            }
        }
        Ok(())
    }
    fn change_nick(&mut self, nick: String, src: i32) -> Result<()> {
        debug!("Contact {} changing nick to {}", self.nick, nick);
        self.store.update_recipient_nick(&self.addr, &nick, src)?;
        self.irc.0.send(Command::NICK(nick))?;
        Ok(())
    }
    fn initialize_watch(&mut self) -> Result<()> {
        debug!("Attempting to WATCH +{}", self.admin);
        self.irc.0.send(Command::Raw("WATCH".into(), vec![format!("+{}", self.admin)], None))?;
        Ok(())
    }
    fn handle_irc_message(&mut self, im: Message) -> Result<()> {
        match im.command {
            Command::Response(Response::RPL_ENDOFMOTD, _, _) |
                Command::Response(Response::ERR_NOMOTD, _, _) => {
                debug!("Contact {} connected", self.addr);
                self.connected = true;
                self.process_messages()?;
                self.initialize_watch()?;
                self.update_away()?;
                self.process_groups()?;
            },
            Command::NICK(nick) => {
                if let Some(from) = im.prefix {
                    let from = from.split("!").collect::<Vec<_>>();
                    if let Some(&from) = from.get(0) {
                        if from == self.nick {
                            self.nick = nick;
                        }
                    }
                }
            },
            Command::PRIVMSG(target, mesg) => {
                if let Some(from) = im.prefix {
                    let from = from.split("!").collect::<Vec<_>>();
                    trace!("{} got PRIVMSG from {:?} to {}: {}", self.addr, from, target, mesg);
                    if from.len() < 1 {
                        return Ok(());
                    }
                    if from[0] != self.admin {
                        self.irc.0.send_notice(from[0], "Message not delivered; you aren't the SMS bridge administrator!")?;
                        return Ok(());
                    }
                    if target == self.nick {
                        debug!("{} -> {}: {}", from[0], self.addr, mesg); 
                        if self.wa_mode {
                            self.wa_tx.unbounded_send(WhatsappCommand::SendDirectMessage(self.addr.clone(), mesg)).unwrap();
                        }
                        else {
                            self.modem_tx.unbounded_send(ModemCommand::SendMessage(self.addr.clone(), mesg)).unwrap();
                        }
                    }
                }
            },
            Command::Raw(cmd, args, suffix) => {
                trace!("Raw response: {} {:?} {:?}", cmd, args, suffix);
                if args.len() < 2 {
                    return Ok(());
                }
                match &cmd as &str {
                    "600" | "604" => { // RPL_LOGON / RPL_NOWON
                        if args[1] == self.admin {
                            debug!("Admin {} is online.", self.admin);
                            if !self.admin_is_online {
                                info!("Admin {} has returned; sending queued messages.", self.admin);
                                self.admin_is_online = true;
                                self.process_messages()?;
                            }
                        }
                    },
                    "601" | "605" => { // RPL_LOGOFF / RPL_NOWOFF
                        if args[1] == self.admin {
                            debug!("Admin {} is offline.", self.admin);
                            if self.admin_is_online {
                                self.admin_is_online = false;
                                warn!("Admin {} has gone offline; queuing messages until their return.", self.admin);
                            }
                        }
                    },
                    _ => {}
                }
            },
            Command::Response(Response::ERR_UNKNOWNCOMMAND, args, suffix) => {
                trace!("Unknown command response: {:?} {:?}", args, suffix);
                if args.len() == 2 && args[1] == "WATCH" {
                    warn!("WATCH not supported by server!");
                }
            },
            Command::ERROR(msg) => {
                return Err(format_err!("Error from server: {}", msg));
            },
            _ => {}
        }
        Ok(())
    }
    pub fn new(recip: Recipient, p: InitParameters<IrcClientConfig>) -> impl Future<Item = Self, Error = Error> {
        let store = p.store;
        let wa_mode = recip.whatsapp;
        let addr = recip.phone_number;
        let (tx, rx) = mpsc::unbounded();
        let modem_tx = p.cm.modem_tx.clone();
        let wa_tx = p.cm.wa_tx.clone();
        let admin = p.cfg2.admin_nick.clone();
        let webirc_password = p.cfg2.webirc_password.clone();
        let cfg = Box::into_raw(Box::new(IrcConfig {
            nickname: Some(recip.nick),
            alt_nicks: Some(vec!["smsirc_fallback".to_string()]),
            realname: Some(addr.to_string()),
            server: Some(p.cfg2.irc_hostname.clone()),
            password: p.cfg2.irc_password.clone(),
            port: p.cfg2.irc_port,
            channels: Some(vec![p.cfg2.irc_channel.clone()]),
            ..Default::default()
        }));
        // DODGY UNSAFE STUFF: The way IrcClient::new_future works is stupid.
        // However, when the future it returns has resolved, it no longer
        // holds a reference to the IrcConfig. Therefore, we fudge a 'static
        // reference here (to satisfy the stupid method), and deallocate
        // it later, when the future has resolved.
        let cfgb: &'static IrcConfig = unsafe { &*cfg };
        let fut = match IrcClient::new_future(p.hdl.clone(), cfgb) {
            Ok(r) => r,
            Err(e) => return Either::B(futures::future::err(e.into()))
        };
        let fut = fut
            .then(move |res| {
                let _ = unsafe { Box::from_raw(cfg) };
                match res {
                    Ok(cli) => {
                        let irc_stream = cli.0.stream();
                        let nick = cli.0.current_nickname().into();
                        Ok(ContactManager {
                            irc: cli,
                            irc_stream,
                            id: false,
                            connected: false,
                            wa_mode,
                            // Assume admin is online to start with
                            admin_is_online: true,
                            presence: None,
                            channels: vec![],
                            addr, store, modem_tx, tx, rx, admin, nick, wa_tx, webirc_password
                        })
                    },
                    Err(e) => {
                        Err(e.into())
                    }
                }
            });
        Either::A(fut)
    }
}
