---
title: Tool Description Style Guide (for LLMs)
---

# Tool Description Style Guide (for LLMs)

- Title: Short imperative (e.g., "Create Card").
- Description: One sentence, active voice, include idempotency and required args.
- Required/Recommended: Call out required keys; suggest safe defaults (e.g., limit ≤ 200).
- Performance Hints: Note when filesystem scanning may occur (query/includeDone).
- Safety Hints: Mention non-idempotent operations (new), and warnings (auto-rename).
- Long-running: Explicitly mark watch-like tools as long-running and not for batch.
- Schema: Provide `inputSchema` (camelCase) and an `x-returns` summary; add `x-examples` with the smallest viable payload.
- Annotations: Prefer lightweight hints such as `idempotentHint`, `readOnlyHint`, `destructiveHint`, `openWorldHint` for clients.

## Template
- Title: <ShortTitle>
- Description: <What it does>. <Idempotency>. Required: <keys>. <Key hints>.
- inputSchema: JSON Schema-like object (types, enums, defaults).
- x-returns: minimal shape of result.
- x-examples: one or two shortest examples.

## Do / Don't
- Do: keep it under ~25 words; include required keys first.
- Do: warn about side effects and performance fallbacks.
- Don't: embed verbose prose, multiple paragraphs, or ambiguous terms.
- Don't: omit constraints (e.g., one parent per child).

