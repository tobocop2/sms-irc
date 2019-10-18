//! Handles database stuff.

use crate::config::Config;
use huawei_modem::pdu::PduAddress;
use serde_json;
use whatsappweb::session::PersistentSession;
use whatsappweb::Jid;
use crate::util::{self, Result};
use chrono::{NaiveDateTime, Utc};
use crate::models::*;
use sequelight::traits::*;
use sequelight::{LightPool, LightConnectionManager};
use sequelight::r2d2::Pool;
use std::sync::Arc;

#[derive(Clone)]
pub struct Store {
    inner: Arc<LightPool>,
}
impl Store {
    pub fn new(cfg: &Config) -> Result<Self> {
        let manager = LightConnectionManager::initialize(cfg.database_url.clone(), &MIGRATIONS)?;
        let pool = Pool::builder()
            .build(manager)?;
        Ok(Self {
            inner: Arc::new(pool)
        })
    }
    // FIXME pass by-value here instead of cloning (well, everywhere really)
    pub fn store_sms_message(&mut self, addr: &PduAddress, pdu: &[u8], csms_data: Option<i32>) -> Result<Message> {
        let db = self.inner.get()?;
        let mut msg = Message {
            id: -1,
            phone_number: addr.clone(),
            pdu: Some(pdu.to_owned()),
            csms_data,
            group_target: None,
            text: None,
            source: Message::SOURCE_SMS,
            ts: Utc::now().naive_utc()
        };
        msg.id = msg.insert_self(&db)?;
        Ok(msg)
    }
    pub fn store_wa_message(&mut self, addr: &PduAddress, text: &str, group_target: Option<i64>, ts: NaiveDateTime) -> Result<Message> {
        let db = self.inner.get()?;
        let mut msg = Message {
            id: -1,
            phone_number: addr.clone(),
            pdu: None,
            csms_data: None,
            group_target,
            text: Some(text.to_owned()),
            source: Message::SOURCE_WA,
            ts
        };
        msg.id = msg.insert_self(&db)?;
        Ok(msg)
    }
    pub fn store_wa_persistence(&mut self, p: PersistentSession) -> Result<()> {
        let db = self.inner.get()?;
        let pdata = serde_json::to_value(&p)?;
        let pdata = PersistenceData {
            rev: 0,
            data: pdata
        };
        pdata.insert_self(&db)?;
        Ok(())
    }
    pub fn store_group(&mut self, jid: &Jid, channel: &str, topic: &str) -> Result<Group> {
        let db = self.inner.get()?;
        let mut grp = Group {
            id: -1,
            jid: jid.clone(),
            channel: channel.to_owned(),
            topic: topic.to_owned()
        };
        grp.id = grp.insert_self(&db)?;
        Ok(grp)
    }
    pub fn update_group_members(&mut self, id: i64, members: Vec<i64>, admins: Vec<i64>) -> Result<()> {
        let mut db = self.inner.get()?;
        let trans = db.transaction()?;
        trans.execute("DELETE FROM group_memberships WHERE group_id = ?", params![id])?;
        for memb in members {
            GroupMembership {
                group_id: id,
                user_id: memb,
                is_admin: admins.contains(&memb)
            }.insert_self(&trans)?;
        }
        trans.commit()?;
        Ok(())
    }
    pub fn get_group_members(&mut self, id: i64) -> Result<Vec<Recipient>> {
        let db = self.inner.get()?;
        let res = Recipient::from_select(&db, "INNER JOIN group_memberships AS gm ON gm.user_id = recipients.id WHERE gm.group_id = ? AND gm.is_admin = false", params![id])?;
        Ok(res)
    }
    pub fn get_group_admins(&mut self, id: i64) -> Result<Vec<Recipient>> {
        let db = self.inner.get()?;
        let res = Recipient::from_select(&db, "INNER JOIN group_memberships AS gm ON gm.user_id = recipients.id WHERE gm.group_id = ? AND gm.is_admin = true", params![id])?;
        Ok(res)
    }
    pub fn update_group_topic(&mut self, id: i64, tpc: &str) -> Result<()> {
        let db = self.inner.get()?;
        db.execute("UPDATE groups SET topic = ? WHERE id = ?", params![id, tpc])?;
        Ok(())
    }
    pub fn get_wa_persistence_opt(&mut self) -> Result<Option<PersistentSession>> {
        let db = self.inner.get()?;
        let pd = PersistenceData::from_select(&db, "WHERE rev = 0", NO_PARAMS)?
            .into_iter()
            .nth(0);
        let res = match pd {
            Some(res) => {
                let res: PersistentSession = serde_json::from_value(res.data)?;
                Some(res)
            },
            None => None
        };
        Ok(res)
    }
    pub fn is_wa_msgid_stored(&mut self, id: &str) -> Result<bool> {
        let db = self.inner.get()?;
        let msgids = WaMessageId::from_select(&db, "WHERE mid = ?", params![id])?;
        Ok(msgids.len() > 0)
    }
    pub fn store_wa_msgid(&mut self, id: String) -> Result<()> {
        let db = self.inner.get()?;
        let new = WaMessageId { mid: id };
        new.insert_self(&db)?;
        Ok(())
    }
    pub fn store_recipient(&mut self, addr: &PduAddress, nick: &str) -> Result<Recipient> {
        let db = self.inner.get()?;
        let mut recip = Recipient {
            id: -1,
            phone_number: addr.clone(),
            nick: nick.to_owned(),
            whatsapp: false,
            avatar_url: None,
            notify: None,
            nicksrc: Recipient::NICKSRC_AUTO
        };
        recip.id = recip.insert_self(&db)?;
        Ok(recip)
    }
    pub fn store_wa_recipient(&mut self, addr: &PduAddress, nick: &str, notify: Option<&str>, nicksrc: i32) -> Result<Recipient> {
        let db = self.inner.get()?;
        let mut recip = Recipient {
            id: -1,
            phone_number: addr.clone(),
            nick: nick.to_owned(),
            whatsapp: true,
            avatar_url: None,
            notify: notify.map(|x| x.to_owned()),
            nicksrc
        };
        recip.id = recip.insert_self(&db)?;
        Ok(recip)
    }
    pub fn update_recipient_notify(&mut self, addr: &PduAddress, n: Option<&str>) -> Result<()> {
        let db = self.inner.get()?;
        let addr = util::normalize_address(addr);
        db.execute("UPDATE recipients SET notify = ? WHERE phone_number = ?", params![n, addr])?;
        Ok(())
    }
    pub fn update_recipient_nick(&mut self, addr: &PduAddress, n: &str, src: i32) -> Result<()> {
        let db = self.inner.get()?;
        let addr = util::normalize_address(addr);
        db.execute("UPDATE recipients SET nick = ?, nicksrc = ? WHERE phone_number = ?", params![n, src, addr])?;
        Ok(())
    }
    pub fn update_recipient_wa(&mut self, addr: &PduAddress, wa: bool) -> Result<()> {
        let db = self.inner.get()?;
        let addr = util::normalize_address(addr);
        db.execute("UPDATE recipients SET whatsapp = ? WHERE phone_number = ?", params![wa, addr])?;
        Ok(())
    }
    pub fn get_recipient_by_nick_opt(&mut self, n: &str) -> Result<Option<Recipient>> {
        let db = self.inner.get()?;
        let res = Recipient::from_select(&db, "WHERE nick = ?", params![n])?
            .into_iter()
            .nth(0);
        Ok(res)
    }
    pub fn get_recipient_by_addr_opt(&mut self, addr: &PduAddress) -> Result<Option<Recipient>> {
        let db = self.inner.get()?;
        let num = util::normalize_address(addr);
        let res = Recipient::from_select(&db, "WHERE phone_number = ?", params![num])?
            .into_iter()
            .nth(0);
        Ok(res)
    }
    pub fn get_all_recipients(&mut self) -> Result<Vec<Recipient>> {
        let db = self.inner.get()?;

        let res = Recipient::from_select(&db, "", NO_PARAMS)?;
        Ok(res)
    }
    pub fn get_all_messages(&mut self) -> Result<Vec<Message>> {
        let db = self.inner.get()?;

        let res = Message::from_select(&db, "ORDER BY ts, id ASC", NO_PARAMS)?;
        Ok(res)
    }
    pub fn get_group_by_id(&mut self, gid: i64) -> Result<Group> {
        let db = self.inner.get()?;

        let res = Group::from_select(&db, "WHERE id = ?", params![gid])?
            .into_iter().nth(0)
            .ok_or(format_err!("group not found"))?;
        Ok(res)
    }
    pub fn get_all_groups(&mut self) -> Result<Vec<Group>> {
        let conn = self.inner.get()?;

        let res = Group::from_select(&conn, "", NO_PARAMS)?;
        Ok(res)
    }
    pub fn get_group_by_jid_opt(&mut self, j: &Jid) -> Result<Option<Group>> {
        let db = self.inner.get()?;
        let j = j.to_string();
        let res = Group::from_select(&db, "WHERE jid = ?", params![j])?
            .into_iter().nth(0);
        Ok(res)
    }
    pub fn get_group_by_chan_opt(&mut self, c: &str) -> Result<Option<Group>> {
        let db = self.inner.get()?;
        let res = Group::from_select(&db, "WHERE channel = ?", params![c])?
            .into_iter().nth(0);
        Ok(res)
    }
    pub fn get_groups_for_recipient(&mut self, addr: &PduAddress) -> Result<Vec<Group>> {
        let db = self.inner.get()?;
        let num = util::normalize_address(addr);
        let res = Group::from_select(&db, "INNER JOIN group_memberships AS gm ON gm.group_id = groups.id INNER JOIN recipients AS r ON gm.user_id = r.id WHERE r.phone_number = ?",
                                     params![num])?;
        Ok(res)
    }
    pub fn get_messages_for_recipient(&mut self, addr: &PduAddress) -> Result<Vec<Message>> {
        let db = self.inner.get()?;
        let num = util::normalize_address(addr);

        let res = Message::from_select(&db, "WHERE phone_number = ? ORDER BY ts, id ASC", params![num])?;
        Ok(res)
    }
    pub fn get_all_concatenated(&mut self, addr: &PduAddress, rf: i32) -> Result<Vec<Message>> {
        let db = self.inner.get()?;
        let num = util::normalize_address(addr);

        let res = Message::from_select(&db, "WHERE csms_data = ? AND phone_number = ?", params![rf, num])?;
        Ok(res)
    }
    pub fn delete_group_with_id(&mut self, i: i64) -> Result<()> {
        let conn = self.inner.get()?;

        let rows_affected = conn.execute("DELETE FROM groups WHERE id = ?", params![i])?;
        if rows_affected == 0 {
            return Err(format_err!("no rows affected deleting gid {}", i));
        }
        Ok(())
    }
    pub fn delete_recipient_with_addr(&mut self, addr: &PduAddress) -> Result<()> {
        let conn = self.inner.get()?;
        let num = util::normalize_address(addr);

        let rows_affected = conn.execute("DELETE FROM recipients WHERE phone_number = ?", params![num])?;
        if rows_affected == 0 {
            return Err(format_err!("no rows affected deleting recip {}", addr));
        }
        Ok(())
    }
    pub fn delete_message(&mut self, mid: i64) -> Result<()> {
        let conn = self.inner.get()?;

        let rows_affected = conn.execute("DELETE FROM messages WHERE id = ?", params![mid])?;
        if rows_affected == 0 {
            return Err(format_err!("no rows affected deleting mid #{}", mid));
        }
        Ok(())
    }
}
