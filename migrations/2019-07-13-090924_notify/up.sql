ALTER TABLE recipients ADD COLUMN notify VARCHAR;
ALTER TABLE messages ADD COLUMN source INT NOT NULL DEFAULT 0;