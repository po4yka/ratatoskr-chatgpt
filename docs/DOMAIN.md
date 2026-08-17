# ChatGPT archive domain model

## Terms

- **Provider export:** immutable official archive plus acquisition, hash, schema, parser, and completeness metadata.
- **Import run:** durable staged parsing/reconciliation attempt.
- **Project:** provider workspace with metadata, instructions, sources/assets, and conversations where present.
- **Conversation:** graph root and provider metadata.
- **Message revision:** immutable message node/version with parent/branch relation.
- **Content part:** text, Markdown, image, file, code, citation, tool call/result, Canvas, or unknown.
- **Asset:** uploaded/generated file or media blob and references.
- **Completeness report:** evidence-based counts, missing relations/assets, unknown variants, and status.

## Invariants

1. Raw exports are immutable and precede normalization.
2. Conversation branches/revisions are never flattened destructively.
3. Unknown provider records survive import.
4. Absence in one snapshot does not equal deletion.
5. Normalized updates create revisions/history rather than rewriting evidence.
6. Archive authority remains separate from Knowledge analysis.
7. Consumer ingestion never depends on undocumented browser sessions.
