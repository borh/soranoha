# Assessment evaluation

Soranoha publishes a work only after establishing that its copyright has
expired in Japan. There are two ways to establish that, and both concern
Japanese law only. What you may do with the published files is stated in
[rights and licensing](../../docs/rights.md). How faithfully the text was
converted is a separate question, answered by
[source accountability](source-accountability.md).

## Reliance on Aozora Bunko's classification

Aozora Bunko marks each work it publishes as copyright-expired or not
(作品著作権フラグ), following its own published rules. For most works,
Soranoha relies on that mark. The record of that reliance holds the catalog
row with the mark, the work's card with its link to the edition, and the file
bundle itself, all kept as the bytes Aozora Bunko served.

Before each release the evaluator fetches the catalog row, the card and the
bundle again. If any of them has changed, is missing or cannot be reached, the
work drops out of the release. A work also drops out if a reviewer has recorded
an exception for it, or a reviewed finding contradicts the mark.

## Independent reviewed findings

A reviewer may instead record findings about each contribution to the
edition: who made it, when that person died, and so on. From those findings the
evaluator derives public-domain standing under one rule, and only this rule.
Every contribution to the edition must satisfy all of the following:

- the contributor died in or before 1967;
- the contribution is attributed to a real name, a registered real name or a
  well-known pseudonym;
- the work is neither a film nor a photograph;
- whether the work was published in the contributor's lifetime or after it is
  established;
- every work it derives from, such as the original of a translation, is itself
  public domain;
- no Allied-powers wartime addition applies to the term;
- the evaluation date is 2018-12-29 or later.

A reviewer may also record a contribution as in copyright or as undetermined.
A work that fails the rule is not thereby protected; it is simply not
published. The statutes behind the rule are quoted in
[the statutory citations](evidence/jp-term-statute-citations.md).

A work is admitted whole or not at all. If the author's copyright has expired
but the translator's has not, the work stays out.

## The release record

Each release manifest includes the hash of an assessment snapshot
(`snh-assessment-snapshot/2`). The snapshot lists every work that was
considered, records which of the two bases admitted it, and gives the SHA-256
of each page the evaluator read. The pages themselves are not published.
Manifests form an append-only chain, so the basis of every admission can be
checked later.
