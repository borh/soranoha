# TEI validation

Two validation layers are specified in the [TEI profile
ODD](../schemas/tei-profile.odd): XML structure is verified with Relax NG, while
semantic constraints (such as non-empty ruby readings, resolved local
references, header metadata, and the structure of preservation records) are
verified with Schematron. The [RNG](../schemas/tei-profile.rng) and
[Schematron](../schemas/tei-profile.sch) are generated from the ODD. Where this
page and the ODD disagree, the ODD is authoritative.

`tei-validation.json` records three layers. `relax_ng` and `schematron` are the
profile's. `well_formed_xml` is the serializer's own check that it wrote
parseable XML; it always passes and does not count towards the overall status.
Each finding has `rule_id`, `severity`, the `layer` it came from, `message`,
`location` and `allowed`. `allowed` is false for exactly the severities that
fail the work, so it and the layer status always agree. In the report, the
profile is identified by the ODD hash. Toolchain metadata includes the ODD,
RNG and Schematron hashes and the generator identity from
[tei-profile-generation.json](../schemas/tei-profile-generation.json);
generation metadata whose hashes do not match the profile is rejected. A
custom profile may have no generator metadata, in which case those fields are
absent rather than null.

A work with findings is published anyway, with its report beside it. Conforming
to the schema means the file matches the profile specification; it does not
guarantee that the conversion preserved every source marker, which is evaluated
under [source accountability](source-accountability.md). Nor does schema
conformance admit the work to a release, which is governed by
[assessment evaluation](assessment-evaluation.md).

Validation runs Jing for Relax NG and ph-schematron's XSLT backend for
Schematron. Schematron rule IDs are `snh-*`. A Relax NG failure is also a
finding, but its `rule_id` is the constant `relax-ng`, because Relax NG
identifies structural violations without associating them with individual rule
names; consumers filtering on the `snh-` prefix therefore disregard them. In the
[extension vocabulary](tei-vocabulary.md), `snh-*` identifiers are listed
alongside the `snh:` attributes they constrain.
