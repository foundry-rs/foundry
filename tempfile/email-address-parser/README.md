# email-address-parser

An RFC 5322, and RFC 6532 compliant email address parser.

You can parse string for email address like this.

```rust
use email_address_parser::EmailAddress;

let email = EmailAddress::parse("foo@bar.com", Some(true)).unwrap();
assert_eq!(email.get_local_part(), "foo");
assert_eq!(email.get_domain(), "bar.com");
```

For an input string that is an invalid email address, it returns `None`.

```rust
use email_address_parser::EmailAddress;

assert!(EmailAddress::parse("test@-iana.org", Some(true)).is_none());
```

To parse an email address with obsolete parts (as per RFC 5322) in it, pass `None` as the second argument to have non-strict parsing.

```rust
let email = EmailAddress::parse("\u{0d}\u{0a} \u{0d}\u{0a} test@iana.org", None);
assert!(email.is_some());
```

## Unicode support

In compliance to [RFC 6532](https://tools.ietf.org/html/rfc6532), it supports parsing, validating, and instantiating email addresses with Unicode characters.

```rust
assert!(format!("{}", EmailAddress.new("foö", "bücher.de")) == "foö@bücher.de");
assert!(format!("{}", EmailAddress.parse("foö@bücher.de")) == "foö@bücher.de");
assert!(EmailAddress.isValid("foö@bücher.de"));
```
