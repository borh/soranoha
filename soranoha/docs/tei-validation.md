# TEI validation

Soranoha owns the [TEI profile ODD](../schemas/tei-profile.odd). It defines two
validation layers: Relax NG validates XML structure, while Schematron enforces
semantic constraints such as nonempty ruby readings, resolved local references,
header metadata, and preservation-record shape. The generated
[RNG](../schemas/tei-profile.rng) and [Schematron](../schemas/tei-profile.sch)
are derived from that ODD. Where this page and the ODD disagree, the ODD is
authoritative.

The `tei-validation.json` output records three layers, not two. `relax_ng` and
`schematron` are the profile's; `well_formed_xml` is the serializer's own
report that it produced parseable XML, always passed, and it does not
contribute to the overall status. Each finding carries `rule_id`, `severity`,
the `layer` it came from, its `message` and `location`, and `allowed`, which
is false exactly for the severities that fail the work: the layer's own status
and this flag apply the same test, so they always agree. Its profile identity
is the ODD hash. Toolchain metadata binds the exact ODD, RNG, and Schematron
hashes, with generator identity from
[tei-profile-generation.json](../schemas/tei-profile-generation.json).

A work with findings is published anyway, with its report beside it. Passing
the schema means the file conforms to the profile. It does not mean the
conversion kept every source marker; that is what
[source accountability](source-accountability.md) reports. And it does not
admit the work; [assessment evaluation](assessment-evaluation.md) does.

Runtime validation uses Jing for Relax NG and ph-schematron's XSLT backend for
Schematron. Schematron rule IDs are `snh-*`. A Relax NG failure is also a
finding, but its `rule_id` is the constant `relax-ng` rather than an `snh-*`
name, because the grammar reports a structural violation without a named rule;
a consumer that filters on the `snh-` prefix drops those. The
[extension vocabulary](tei-vocabulary.md) documents the `snh-*` identifiers
alongside the `snh:` attributes they constrain.
