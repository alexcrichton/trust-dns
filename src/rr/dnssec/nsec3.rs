/*
 * Copyright (C) 2015 Benjamin Fry <benjaminfry@me.com>
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */
use std::io::Write;
use openssl::crypto::hash;

use ::error::*;
use ::rr::dnssec::DigestType;
use ::rr::Name;
use ::serialize::binary::{BinEncoder, BinSerializable};

// RFC 5155                         NSEC3                        March 2008
//
// 11.  IANA Considerations
//
//    Although the NSEC3 and NSEC3PARAM RR formats include a hash algorithm
//    parameter, this document does not define a particular mechanism for
//    safely transitioning from one NSEC3 hash algorithm to another.  When
//    specifying a new hash algorithm for use with NSEC3, a transition
//    mechanism MUST also be defined.
//
//    This document updates the IANA registry "DOMAIN NAME SYSTEM
//    PARAMETERS" (http://www.iana.org/assignments/dns-parameters) in sub-
//    registry "TYPES", by defining two new types.  Section 3 defines the
//    NSEC3 RR type 50.  Section 4 defines the NSEC3PARAM RR type 51.
//
//    This document updates the IANA registry "DNS SECURITY ALGORITHM
//    NUMBERS -- per [RFC4035]"
//    (http://www.iana.org/assignments/dns-sec-alg-numbers).  Section 2
//    defines the aliases DSA-NSEC3-SHA1 (6) and RSASHA1-NSEC3-SHA1 (7) for
//    respectively existing registrations DSA and RSASHA1 in combination
//    with NSEC3 hash algorithm SHA1.
//
//    Since these algorithm numbers are aliases for existing DNSKEY
//    algorithm numbers, the flags that exist for the original algorithm
//    are valid for the alias algorithm.
//
//    This document creates a new IANA registry for NSEC3 flags.  This
//    registry is named "DNSSEC NSEC3 Flags".  The initial contents of this
//    registry are:
//
//      0   1   2   3   4   5   6   7
//    +---+---+---+---+---+---+---+---+
//    |   |   |   |   |   |   |   |Opt|
//    |   |   |   |   |   |   |   |Out|
//    +---+---+---+---+---+---+---+---+
//
//       bit 7 is the Opt-Out flag.
//
//       bits 0 - 6 are available for assignment.
//
//    Assignment of additional NSEC3 Flags in this registry requires IETF
//    Standards Action [RFC2434].
//
//    This document creates a new IANA registry for NSEC3PARAM flags.  This
//    registry is named "DNSSEC NSEC3PARAM Flags".  The initial contents of
//    this registry are:
//
//      0   1   2   3   4   5   6   7
//    +---+---+---+---+---+---+---+---+
//    |   |   |   |   |   |   |   | 0 |
//    +---+---+---+---+---+---+---+---+
//
//       bit 7 is reserved and must be 0.
//
//       bits 0 - 6 are available for assignment.
//
//    Assignment of additional NSEC3PARAM Flags in this registry requires
//    IETF Standards Action [RFC2434].
//
//    Finally, this document creates a new IANA registry for NSEC3 hash
//    algorithms.  This registry is named "DNSSEC NSEC3 Hash Algorithms".
//    The initial contents of this registry are:
//
//       0 is Reserved.
//
//       1 is SHA-1.
//
//       2-255 Available for assignment.
//
//    Assignment of additional NSEC3 hash algorithms in this registry
//    requires IETF Standards Action [RFC2434].
#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub enum Nsec3HashAlgorithm {
  SHA1,
}

impl Nsec3HashAlgorithm {
  /// http://www.iana.org/assignments/dns-sec-alg-numbers/dns-sec-alg-numbers.xhtml
  pub fn from_u8(value: u8) -> DecodeResult<Self> {
    match value {
      1  => Ok(Nsec3HashAlgorithm::SHA1),
      // TODO: where/when is SHA2?
      _ => Err(DecodeErrorKind::UnknownAlgorithmTypeValue(value).into()),
    }
  }

  // Laurie, et al.              Standards Track                    [Page 14]
  //
  // RFC 5155                         NSEC3                        March 2008
  //
  // Define H(x) to be the hash of x using the Hash Algorithm selected by
  //    the NSEC3 RR, k to be the number of Iterations, and || to indicate
  //    concatenation.  Then define:
  //
  //       IH(salt, x, 0) = H(x || salt), and
  //
  //       IH(salt, x, k) = H(IH(salt, x, k-1) || salt), if k > 0
  //
  //    Then the calculated hash of an owner name is
  //
  //       IH(salt, owner name, iterations),
  //
  //    where the owner name is in the canonical form, defined as:
  //
  //    The wire format of the owner name where:
  //
  //    1.  The owner name is fully expanded (no DNS name compression) and
  //        fully qualified;
  //
  //    2.  All uppercase US-ASCII letters are replaced by the corresponding
  //        lowercase US-ASCII letters;
  //
  //    3.  If the owner name is a wildcard name, the owner name is in its
  //        original unexpanded form, including the "*" label (no wildcard
  //        substitution);
  pub fn hash(&self, salt: &[u8], name: &Name, iterations: u16) -> Vec<u8> {
    match *self {
      // if there ever is more than just SHA1 support, this should be a genericized method
      Nsec3HashAlgorithm::SHA1 => {
        let mut buf: Vec<u8> = Vec::new();
        {
          let mut encoder: BinEncoder = BinEncoder::new(&mut buf);
          encoder.set_canonical_names(true);
          name.emit(&mut encoder).expect("could not encode Name");
        }

        Self::sha1_recursive_hash(salt, buf, iterations)
      },
    }
  }

  // until there is another supported algorithm, just hardcoded to this.
  fn sha1_recursive_hash(salt: &[u8], bytes: Vec<u8>, iterations: u16) -> Vec<u8> {
    let mut hasher: hash::Hasher = hash::Hasher::new(DigestType::SHA1.to_hash());

    if iterations > 0 {
      hasher.write_all(&Self::sha1_recursive_hash(salt, bytes, iterations - 1)).expect("hasher failed");
    } else {
      hasher.write_all(&bytes).expect("hasher failed");
    }
    hasher.write_all(salt).expect("hasher failed");
    hasher.finish()
  }
}

impl From<Nsec3HashAlgorithm> for u8 {
  fn from(a: Nsec3HashAlgorithm) -> u8 {
    match a {
      Nsec3HashAlgorithm::SHA1 => 1,
    }
  }
}

#[test]
fn test_hash() {

  let name = Name::new().label("www").label("example").label("com");
  let salt: Vec<u8> = vec![1,2,3,4];

  assert_eq!(Nsec3HashAlgorithm::SHA1.hash(&salt, &name, 0).len(), 20);
  assert_eq!(Nsec3HashAlgorithm::SHA1.hash(&salt, &name, 1).len(), 20);
  assert_eq!(Nsec3HashAlgorithm::SHA1.hash(&salt, &name, 3).len(), 20);
}

#[test]
fn test_known_hashes() {
  // H(example)       = 0p9mhaveqvm6t7vbl5lop2u3t2rp3tom
  assert_eq!(hash_with_base32("example"), "0p9mhaveqvm6t7vbl5lop2u3t2rp3tom");

  // H(a.example)     = 35mthgpgcu1qg68fab165klnsnk3dpvl
  assert_eq!(hash_with_base32("a.example"), "35mthgpgcu1qg68fab165klnsnk3dpvl");

  // H(ai.example)    = gjeqe526plbf1g8mklp59enfd789njgi
  assert_eq!(hash_with_base32("ai.example"), "gjeqe526plbf1g8mklp59enfd789njgi");

  // H(ns1.example)   = 2t7b4g4vsa5smi47k61mv5bv1a22bojr
  assert_eq!(hash_with_base32("ns1.example"), "2t7b4g4vsa5smi47k61mv5bv1a22bojr");

  // H(ns2.example)   = q04jkcevqvmu85r014c7dkba38o0ji5r
  assert_eq!(hash_with_base32("ns2.example"), "q04jkcevqvmu85r014c7dkba38o0ji5r");

  // H(w.example)     = k8udemvp1j2f7eg6jebps17vp3n8i58h
  assert_eq!(hash_with_base32("w.example"), "k8udemvp1j2f7eg6jebps17vp3n8i58h");

  // H(*.w.example)   = r53bq7cc2uvmubfu5ocmm6pers9tk9en
  assert_eq!(hash_with_base32("*.w.example"), "r53bq7cc2uvmubfu5ocmm6pers9tk9en");

  // H(x.w.example)   = b4um86eghhds6nea196smvmlo4ors995
  assert_eq!(hash_with_base32("x.w.example"), "b4um86eghhds6nea196smvmlo4ors995");

  // H(y.w.example)   = ji6neoaepv8b5o6k4ev33abha8ht9fgc
  assert_eq!(hash_with_base32("y.w.example"), "ji6neoaepv8b5o6k4ev33abha8ht9fgc");

  // H(x.y.w.example) = 2vptu5timamqttgl4luu9kg21e0aor3s
  assert_eq!(hash_with_base32("x.y.w.example"), "2vptu5timamqttgl4luu9kg21e0aor3s");

  // H(xx.example)    = t644ebqk9bibcna874givr6joj62mlhv
  assert_eq!(hash_with_base32("xx.example"), "t644ebqk9bibcna874givr6joj62mlhv");
}

#[cfg(test)]
fn hash_with_base32(name: &str) -> String {
  use data_encoding::base32hex;

  // NSEC3PARAM 1 0 12 aabbccdd
  let known_name = Name::parse(name, Some(&Name::new())).unwrap();
  let known_salt = [0xAAu8, 0xBBu8, 0xCCu8, 0xDDu8,];
  let hash = Nsec3HashAlgorithm::SHA1.hash(&known_salt, &known_name, 12);
  base32hex::encode(&hash).to_lowercase()
}
