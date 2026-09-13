# Zenodo deposits and ORCID anchoring

Soranoha uses two distinct Zenodo deposit types:

| | Trust anchor | Snapshot deposit |
|---|---|---|
| Purpose | Authenticates which public key holds which role | Archives a release's bytes independently of the publication channel |
| Frequency | Once, before the first signed release | Quarterly |
| Contents | The two role public keys and their role assignment | The then-current signed manifest, the records it names, and the two bulk archives served for that release |
| Discovery | Listed as a work on the owner's ORCID record | Cited by version DOI in each work's citation, when the serving release has one |

Both deposits are made by the publication owner under the Zenodo account
`borh`, from a machine that is not release CI. Zenodo credentials are never
available to the release workflow. The release key signs manifests
automatically, and adding archive credentials to the same automation would put
the ability to publish and the ability to make something citable in one place.

Cite a **version** DOI, never the concept DOI, wherever the DOI has to resolve
to fixed bytes. A concept DOI resolves to the latest version, so what it names
changes underneath a citation.

## Deposit the anchor and record it on ORCID before the first release is signed

Each step below produces the input the next one needs, and the last three
cannot be reordered without either an unverifiable release or a chain that has
to be abandoned rather than corrected.

1. The [key ceremony](key-ceremony.md) produces `release.pub` and
   `governance.pub`.
2. The anchor is deposited, and its **version** DOI recorded.
3. That DOI is added as a work on <https://orcid.org/0000-0003-2246-8774>.
4. Only then is the first release signed.

A release signed ahead of the ORCID entry is not verifiable by the procedure
the protocol specifies, and the chain is append-only, so there is no amendment
that fixes it.

## The trust anchor

The anchor is the only place pinned key bytes and fingerprints are published.
It authenticates the *role assignment*, not a list of keys:

```
RELEASE    = {K_release}
GOVERNANCE = {K_governance}
```

A flat list would be a different and weaker claim, because it would not say
that the online release key is not authorized for governance. The role sets are
disjoint, and the kernel rejects an overlapping or un-roled pinned
configuration.

### Contents

Deposit exactly three files: the two 65-byte `.pub` files the ceremony
produced, unchanged, and a role-assignment statement. Generate the statement
rather than typing it, so that the fingerprints in the anchor cannot disagree
with the keys beside them:

Run this in the directory holding the two `.pub` files.

```sh
set -eu

# Check both keys before writing anything. A missing or malformed file must
# stop the run: a command substitution's failure does not, so the check
# cannot live inside the statement that generates the document.
for file in release.pub governance.pub; do
  test "$(wc -c < "$file")" -eq 65
  tr -d '\n' < "$file" | grep -qE '^[0-9a-f]{64}$'
done

fingerprint() {
  printf "$(printf '%s' "$(tr -d '\n' < "$1")" | sed 's/../\\x&/g')" \
    | sha256sum | cut -d' ' -f1
}

{
  printf 'soranoha snh/1 trust anchor\n\n'
  for role in RELEASE GOVERNANCE; do
    file="$(printf '%s' "$role" | tr 'A-Z' 'a-z').pub"
    printf '%s\n  key         %s\n  fingerprint %s\n\n' \
      "$role" "$(tr -d '\n' < "$file")" "$(fingerprint "$file")"
  done
  printf 'These two roles are disjoint. The release key is not authorized for\n'
  printf 'governance events, and the governance key does not sign release\n'
  printf 'manifests. No other key is pinned for this chain, and the pinned set\n'
  printf 'cannot change.\n'
} > role-assignment.txt

cat role-assignment.txt
```

Read the result against the fingerprints recorded by hand during the ceremony
before depositing it. The guard above exists because the unguarded form fails
in the worst available way: with one `.pub` file missing it wrote a complete
and plausible anchor whose governance key was empty and whose fingerprint was
`e3b0c442...`, the sha256 of no bytes, with the error on stderr and the
document on stdout. An anchor is the permanent authority for the chain, so it
is the last artifact that should be able to come out looking finished when it
is not.

The fingerprint is the sha256 of the **decoded 32 raw key bytes**, which is why
the hex is turned back into bytes before hashing rather than being hashed as
text. Hashing the 65-byte file instead produces a plausible digest that matches
nothing, and the ceremony record is where that error would be caught.

Do not deposit private keys, medium passphrases, or the custody inventory. Do
not deposit a second copy of the fingerprints anywhere else: the protocol
publishes them in one place so that no two copies can drift. Do not deposit the
serving domain or any other location, because deposit files are immutable and a
frozen file should not name something that can move.

### The ORCID entry

Add the anchor's version DOI as a work on the owner's ORCID record. That entry
is the pre-release discovery channel: a verifier who knows only the project's
name reaches the ORCID record, follows the one pointer to the anchor, and
obtains the role assignment from an archive the publication channel does not
control.

The anchor is deposited once. Later releases do not update it, because the
pinned set never changes: a lost medium leaves governance operating on the
remaining copy, and a compromised role halts that role rather than rotating it.

## The snapshot deposit

Deposits are quarterly, not per release. Each one carries the manifest that is
current when it is made, and doubles as an independent authorship checkpoint:
the archive is a second, separately credentialed record of what this project
published and when, held somewhere the publication channel does not control.

A release is citable the moment it is reachable, and that does not depend on a
deposit. The citable name of a release is its manifest id, which is the sha256
of bytes anyone can check against the chain. A DOI names an archived copy; it
is a convenience, not the identity.

Releases published between two snapshots are therefore not individually
archived on Zenodo. Their bytes remain in the append-only chain, which is what
a verifier reads, and they carry no DOI. `release-doi` is optional throughout
for exactly this reason, and a citation simply omits the DOI line when the
serving release has none.

Deposit the manifest and the records it names, along with the two served bulk
archives (TEI and plaintext). The manifest and constituent records provide
cryptographic verification, while the bulk archives ensure published texts
remain retrievable independently of the serving site. The archives contain the
exact served bytes, with `SHA256SUMS` matching the site's downloads and each
archive's `catalog.csv` naming the release head and version DOI. Together they
total under 1 GB, well within Zenodo's 50 GB per-record limit. Do not include
role public keys; they belong exclusively to the trust anchor.

### Assemble it

Because `catalog.csv` in each bulk archive records `release-doi`, the DOI must
be reserved on the Zenodo draft before running `serving-activate`. Assemble the
deposit only after export. If a tree was exported without the DOI, re-export it
rather than modifying the archive.

The manifest, signature, and three constituent records are extracted from a
chain clone; the two bulk archives are copied from the serving root's
`current/bulk/` tree. The assembly script verifies each chain file against its
digest filename and checks that all rows in each archive's `catalog.csv` match
the manifest release head:

```sh
set -eu
clone=/path/to/chain-clone
tree=/path/to/serve-root/current
out="$PWD/deposit"
mkdir -p "$out"

hex=$(tr -d '\n' < "$clone/releases/HEAD")
test "$(sha256sum < "$clone/releases/$hex.json" | cut -d' ' -f1)" = "$hex"
cp "$clone/releases/$hex.json" "$clone/releases/$hex.sig" "$out/"

for field in .catalog .admission.assessment_snapshot .admission.admission_report; do
  id=$(jq -r "$field" "$out/$hex.json")
  blob=${id##*:}
  src="$clone/blobs/sha256/$(printf '%s' "$blob" | cut -c1-2)/$blob"
  test "$(sha256sum < "$src" | cut -d' ' -f1)" = "$blob"
  cp "$src" "$out/$blob"
done

for zip in soranoha-tei.zip soranoha-plaintext.zip; do
  cp "$tree/bulk/$zip" "$out/"
  test "$(unzip -p "$out/$zip" catalog.csv | tail -n +2 | grep -vc "\"$hex\"")" = 0
done

(cd "$out" && sha256sum -- * > SHA256SUMS)
cat "$out/SHA256SUMS"
```

The three constituent records retain their bare content-addressed digest names,
referenced by the manifest's `catalog`, `admission.assessment_snapshot`, and
`admission.admission_report` fields. The bulk archives retain their standard
filenames (`soranoha-tei.zip` and `soranoha-plaintext.zip`), allowing users to
verify site downloads directly against deposited checksums.

### Upload it

Include the corpus release name in Zenodo's version field and the maturity label
in the description, matching the served site. Because these editorial labels are
omitted from signed manifests, the Zenodo deposit serves as their archived record.

Set the record's licence to CC0-1.0, adding Creative Commons Attribution if the
release includes CC BY works. CC0 applies to Soranoha's encoding layer;
underlying work standings are not Soranoha's to license. State the admitted
works' copyright standings in the deposit description, derived from the
manifest:

```sh
jq -r '.works | map(.rights) | group_by(.) | map("\(.[0]) \(length)") | .[]' "$out/$hex.json"
```

If all works are in the public domain, state this directly. If works under
Creative Commons Attribution are included, state their count and list the
corresponding CC BY licence in Zenodo's metadata so attribution conditions are
visible without downloading archives. In all cases, attribute Aozora Bunko in
the description for the bibliographic metadata in `catalog.csv` (licensed CC BY
4.0).

The upload itself follows Zenodo's procedure, using the deposit form or REST
API under the `borh` account. Each snapshot deposit is created as a **new
version** of the initial deposit rather than a separate record, sharing a single
concept DOI while assigning a unique version DOI to each release.

Do not rely on post-publication file replacement or metadata edits: assemble all
files, including bulk archives, before publishing so the version DOI resolves
immediately to fixed bytes.

### Recording the DOI

The DOI is not in the chain. The signed catalog carries no DOI and no provider
reference, and the manifest schema is closed, so a DOI field could never be
removed once added. A DOI describes a deposit rather than a release: the same
release could be deposited twice, deposited elsewhere, or not deposited at all.
Reserving a DOI and signing it into an immutable manifest fails permanently
when the deposit is abandoned, whereas a DOI held outside simply does not
render.

So the DOI is deployment configuration. Set `release-doi` in
`/etc/soranoha/publisher.json`, or pass `--release-doi`, and run
`serving-activate`. Zenodo assigns the DOI to the unpublished draft on request,
which is what lets activation run, and the archives be assembled from its
tree, before the deposit is published.

The value is the **bare DOI**, not a link. It is checked against
`10.<4-9 digits>/<no whitespace>` before it reaches any citation, so
`10.5281/zenodo.1234567` is accepted and `https://doi.org/10.5281/zenodo.1234567`
is refused at activation. Refusing there costs one corrected flag; the
alternative renders a dead pointer into the citation record of every work.

Activation exports the tree, and the citations it generates name the DOI, so
the DOI is an input to the export rather than a label applied after it. Setting
or changing it for a commit that was already exported makes the reuse check
fail closed with `serving-tree-mismatch`: the existing tree's citations name a
different DOI. Remove that tree and re-export. Activation is otherwise
idempotent, so a DOI recorded late is recovered by re-running it.

Activation therefore waits on Zenodo being reachable. For a research corpus
that delay is acceptable, and recording it as policy here is what keeps it from
arriving as a surprise mid-release.

## Verifying deposit integrity

Because the deposit includes manifest bytes, the correspondence between a DOI
and a release is verifiable independently of the publication chain using
`openssl`, coreutils, and the release key from the trust anchor:

```sh
set -eu
hex=...    # the manifest's digest, which is also its .json file name
pub=...    # the release key's 64 hex characters, from the trust anchor

test "$(sha256sum < "$hex.json" | cut -d' ' -f1)" = "$hex"
printf "$(printf '302a300506032b6570032100%s' "$pub" | sed 's/../\\x&/g')" > release.pub.der
printf 'snh-manifest-sig/1:%s' "$hex" > manifest.msg
openssl pkeyutl -verify -rawin -pubin -inkey release.pub.der -keyform DER \
  -in manifest.msg -sigfile "$hex.sig"
```

`302a300506032b6570032100` is the fixed prefix of an Ed25519 public key in DER,
so the `printf` turns 64 hex characters into the 44-byte key file OpenSSL reads.
The message is exact ASCII with no trailing newline: 19 prefix characters plus
64 hex characters is 83 bytes, and a trailing newline produces a verification
failure rather than a warning.

A DOI that names some other release fails the digest check. A manifest that was
not signed by the pinned release key fails the signature check. The three
records verify the same way, by hashing each file and comparing the digest with
the id the manifest gives it. The mapping from DOI to release is configuration,
but the claim it makes is checkable against the chain.

The kernel's `archive-verify` is not this check. It runs the chain verifier over
an archived *repository*, so it needs a git view rather than a directory of
loose files, and a Zenodo deposit is the latter.

## See also

- [Key ceremony and offline governance signing](key-ceremony.md), which
  produces the bytes the anchor deposits.
- [Citing Soranoha](citation.md), for how the version DOI appears in a
  citation.
- [Archival](archival.md), the other independent record. A deposit holds loose
  manifest bytes; the archive holds the repository they were published from.
