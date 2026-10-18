# Assessment evaluation

Soranoha publishes a work only after confirming that its copyright has
expired in Japan. There are two ways to confirm that, and both apply Japanese
law only. [Rights and licensing](../../docs/rights.md) covers what you may do
with the published files. [Source accountability](source-accountability.md)
covers how faithfully the text was converted.

## Reliance on Aozora Bunko's classification

Aozora Bunko marks each work it publishes as copyright-expired or not
(作品著作権フラグ), following its own published rules. For most works,
Soranoha relies on that mark. The reliance record keeps the catalog row with
the mark, the work's card with its link to the edition, and the file bundle, as
the bytes Aozora Bunko served.

Before each release the evaluator fetches the catalog row, the card and the
bundle again. If any of them has changed, is missing or cannot be reached, the
work drops out of the release. A work also drops out if a reviewer has recorded
an exception for it, or a reviewed finding contradicts the mark.

## Independent reviewed findings

A reviewer may instead record findings about each contribution to the
edition: who made it, when that person died, and so on. From those findings the
evaluator derives public-domain standing under a single rule. Every
contribution to the edition must satisfy all of the following:

- the contributor died in or before 1967;
- the contribution is attributed to a real name, a registered real name or a
  well-known pseudonym;
- the work is neither a film nor a photograph;
- it is known whether the work was first published in the contributor's
  lifetime or after it;
- every work it derives from, such as the original of a translation, is itself
  public domain;
- no Allied-powers wartime addition applies to the term;
- the evaluation date is 2018-12-29 or later.

A reviewer may also record a contribution as in copyright or as undetermined.
Failing the rule does not mean a work is protected, only that it is not
published. The statutes behind the rule are quoted in
[the statutory citations](evidence/jp-term-statute-citations.md).

A work is admitted whole or not at all. If the author's copyright has expired
but the translator's has not, the work stays out.

## The release record

Each release manifest includes the hash of an assessment snapshot
(`snh-assessment-snapshot/2`). The snapshot contains every work considered,
indicating which of the two bases admitted it and the SHA-256 of each page the
evaluator read. The pages themselves are not published. The manifests form an
append-only chain, allowing the basis of every admission to be verified
subsequently.
