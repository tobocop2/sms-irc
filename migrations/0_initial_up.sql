CREATE TABLE recipients (
	id INTEGER PRIMARY KEY,
	phone_number TEXT NOT NULL,
	nick TEXT NOT NULL,
	whatsapp BOOL NOT NULL,
	avatar_url TEXT,
	notify TEXT,
	nicksrc INT NOT NULL
);
CREATE INDEX recipients_phone_number ON recipients (phone_number);
CREATE INDEX recipients_nick ON recipients (nick);

CREATE TABLE groups (
	id INTEGER PRIMARY KEY,
	jid TEXT UNIQUE NOT NULL,
	channel TEXT UNIQUE NOT NULL,
	topic TEXT NOT NULL
);

CREATE TABLE messages (
	id INTEGER PRIMARY KEY,
	phone_number TEXT NOT NULL,
	pdu BLOB,
	csms_data INT,
	group_target INT REFERENCES groups,
	text TEXT,
	source INT NOT NULL,
	ts TEXT NOT NULL
);
CREATE INDEX messages_phone_number ON messages (phone_number);
CREATE INDEX messages_csms_data ON messages (csms_data);
CREATE INDEX messages_group_target ON messages (group_target);

CREATE TABLE group_memberships (
	group_id INT NOT NULL REFERENCES groups,
	user_id INT NOT NULL REFERENCES recipients,
	is_admin BOOL NOT NULL,
	UNIQUE(group_id, user_id)
);

CREATE TABLE wa_persistence (
	rev INT UNIQUE NOT NULL,
	data TEXT NOT NULL
);

CREATE TABLE wa_msgid (
	mid TEXT UNIQUE NOT NULL
);
