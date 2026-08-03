-- 0002_drift_dimensions.sql
-- Phase 2.5.1: 多维 drift（face / style）+ voice 已有。
-- 加 face_embed / style_embed 列存每个 persona_version 的 anchor embedding。
-- drift eval = base.<dim>_embed cosine target.<dim>_embed。
--
-- 注意：voice_embed 在 0001 已建；这里只加 face + style。
-- person 维度的 embed 暂用 descriptor 文本算（不下沉到 schema）。

ALTER TABLE persona_versions ADD COLUMN face_embed BLOB;
ALTER TABLE persona_versions ADD COLUMN face_embed_dim INTEGER;
ALTER TABLE persona_versions ADD COLUMN face_embed_sha256 TEXT;
ALTER TABLE persona_versions ADD COLUMN style_embed BLOB;
ALTER TABLE persona_versions ADD COLUMN style_embed_dim INTEGER;
ALTER TABLE persona_versions ADD COLUMN style_embed_sha256 TEXT;
