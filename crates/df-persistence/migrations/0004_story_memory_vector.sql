-- STORY-MEM-02: the semantic half of the story memory, applied only where pgvector exists.
-- A failure here leaves the full text search of 0003 working and is reported at start-up.
CREATE EXTENSION IF NOT EXISTS vector;

ALTER TABLE df_story.story_memory ADD COLUMN IF NOT EXISTS embedding vector(1536);
-- Every vector is versioned by the model that made it; a changed model re-embeds the row.
ALTER TABLE df_story.story_memory ADD COLUMN IF NOT EXISTS embedding_model text
    CHECK (embedding_model IS NULL OR char_length(embedding_model) BETWEEN 1 AND 64);
ALTER TABLE df_story.story_memory ADD COLUMN IF NOT EXISTS embedded_at timestamptz;

CREATE INDEX IF NOT EXISTS story_memory_embedding_hnsw
    ON df_story.story_memory USING hnsw (embedding vector_cosine_ops);
CREATE INDEX IF NOT EXISTS story_memory_unembedded
    ON df_story.story_memory (event_id) WHERE embedding IS NULL;
