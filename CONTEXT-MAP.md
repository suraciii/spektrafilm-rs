# Context Map

## Contexts

- [Film simulation](./CONTEXT.md) — project-wide film workflow language and shared domain terms; [Dye grain](./specs/film-grain/dye/CONTEXT.md) extends this vocabulary for the Dye bounded context.
- [Measurement instrument](./specs/lab/CONTEXT.md) — the delivered-image measurement and structured-record language of `spektrafilm-lab`, and the report that reads its records.

## Relationships

- **Film simulation -> Dye grain**: Dye grain uses the shared film workflow vocabulary and adds terms specific to conditional dye formation and optical measurement.
- **Film simulation -> Measurement instrument**: The instrument reads the pixels a film workflow delivered. It adds no exposure, density, or development term, and it never renders, so nothing in the film workflow depends on it.
