#[cfg(target_arch = "wasm32")]
extern crate console_error_panic_hook;
extern crate pest;
extern crate pest_derive;
use pest::{iterators::Pairs, Parser};
use std::fmt;
use std::hash::Hash;
use std::str::FromStr;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

/// Options for parsing.
///
/// The is only one available option so far `is_lax` which can be set to
/// `true` or `false` to  enable/disable obsolete parts parsing.
/// The default is `false`.
#[cfg_attr(target_arch = "wasm32", wasm_bindgen)]
#[derive(Debug,Clone)]
pub struct ParsingOptions {
    pub is_lax: bool,
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen)]
impl ParsingOptions {
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen(constructor))]
    pub fn new(is_lax: bool) -> ParsingOptions {
        ParsingOptions { is_lax }
    }
}

impl Default for ParsingOptions {
    fn default() -> Self {
        ParsingOptions::new(false)
    }
}

/// Allows conversion from string slices (&str) to EmailAddress using the FromStr trait.
/// This wraps around `EmailAddress::parse` using the default `ParsingOptions`.
///
/// # Examples
/// ```
/// use email_address_parser::EmailAddress;
/// use std::str::FromStr;
///
/// const input_address : &str = "string@slice.com";
///
/// let myaddr : EmailAddress = input_address.parse().expect("could not parse str into EmailAddress");
/// let myotheraddr = EmailAddress::from_str(input_address).expect("could create EmailAddress from str");
///
/// assert_eq!(myaddr, myotheraddr);
/// ```
impl FromStr for EmailAddress {
    type Err = fmt::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let opts = ParsingOptions::default();
        if let Some(email) = EmailAddress::parse(s, Some(opts)) {
            Ok(email)
        } else {
            Err(fmt::Error)
        }
    }
}

#[derive(Parser)]
#[grammar = "rfc5322.pest"]
struct RFC5322;

/// Email address struct.
///
/// # Examples
/// ```
/// use email_address_parser::EmailAddress;
///
/// assert!(EmailAddress::parse("foo@-bar.com", None).is_none());
/// let email = EmailAddress::parse("foo@bar.com", None);
/// assert!(email.is_some());
/// let email = email.unwrap();
/// assert_eq!(email.get_local_part(), "foo");
/// assert_eq!(email.get_domain(), "bar.com");
/// assert_eq!(format!("{}", email), "foo@bar.com");
/// ```
#[cfg_attr(target_arch = "wasm32", wasm_bindgen)]
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct EmailAddress {
    local_part: String,
    domain: String,
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen)]
impl EmailAddress {
    #![warn(missing_docs)]
    #![warn(rustdoc::missing_doc_code_examples)]

    /// This is a WASM wrapper over EmailAddress::new that panics.
    /// If you are using this lib from Rust then consider using EmailAddress::new.
    ///
    /// # Examples
    /// ```
    /// use email_address_parser::EmailAddress;
    ///
    /// let email = EmailAddress::_new("foo", "bar.com", None);
    /// ```
    ///
    /// # Panics
    ///
    /// This method panics if the local part or domain is invalid.
    ///
    /// ```rust,should_panic
    /// use email_address_parser::EmailAddress;
    ///
    /// EmailAddress::_new("foo", "-bar.com", None);
    /// ```
    #[doc(hidden)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen(constructor))]
    pub fn _new(local_part: &str, domain: &str, options: Option<ParsingOptions>) -> EmailAddress {
        #[cfg(target_arch = "wasm32")]
        console_error_panic_hook::set_once();
        match EmailAddress::new(local_part, domain, options) {
            Ok(instance) => instance,
            Err(message) => panic!("{}", message),
        }
    }

    /// Parses a given string as an email address.
    ///
    /// Accessible from WASM.
    ///
    /// Returns `Some(EmailAddress)` if the parsing is successful, else `None`.
    /// # Examples
    /// ```
    /// use email_address_parser::*;
    ///
    /// // strict parsing
    /// let email = EmailAddress::parse("foo@bar.com", None);
    /// assert!(email.is_some());
    /// let email = email.unwrap();
    /// assert_eq!(email.get_local_part(), "foo");
    /// assert_eq!(email.get_domain(), "bar.com");
    ///
    /// // non-strict parsing
    /// let email = EmailAddress::parse("\u{0d}\u{0a} \u{0d}\u{0a} test@iana.org", Some(ParsingOptions::new(true)));
    /// assert!(email.is_some());
    ///
    /// // parsing invalid address
    /// let email = EmailAddress::parse("test@-iana.org", Some(ParsingOptions::new(true)));
    /// assert!(email.is_none());
    /// let email = EmailAddress::parse("test@-iana.org", Some(ParsingOptions::new(true)));
    /// assert!(email.is_none());
    /// let email = EmailAddress::parse("test", Some(ParsingOptions::new(true)));
    /// assert!(email.is_none());
    /// let email = EmailAddress::parse("test", Some(ParsingOptions::new(true)));
    /// assert!(email.is_none());
    /// ```
    pub fn parse(input: &str, options: Option<ParsingOptions>) -> Option<EmailAddress> {
        let instantiate = |mut parsed: pest::iterators::Pairs<Rule>| {
            let mut parsed = parsed
                .next()
                .unwrap()
                .into_inner()
                .next()
                .unwrap()
                .into_inner();
            Some(EmailAddress {
                local_part: String::from(parsed.next().unwrap().as_str()),
                domain: String::from(parsed.next().unwrap().as_str()),
            })
        };
        match EmailAddress::parse_core(input, options) {
            Some(parsed) => instantiate(parsed),
            None => None,
        }
    }
    /// Validates if the given `input` string is an email address or not.
    ///
    /// Returns `true` if the `input` is valid, `false` otherwise.
    /// Unlike the `parse` method, it does not instantiate an `EmailAddress`.
    /// # Examples
    /// ```
    /// use email_address_parser::*;
    ///
    /// // strict validation
    /// assert!(EmailAddress::is_valid("foo@bar.com", None));
    ///
    /// // non-strict validation
    /// assert!(EmailAddress::is_valid("\u{0d}\u{0a} \u{0d}\u{0a} test@iana.org", Some(ParsingOptions::new(true))));
    ///
    /// // invalid address
    /// assert!(!EmailAddress::is_valid("test@-iana.org", Some(ParsingOptions::new(true))));
    /// assert!(!EmailAddress::is_valid("test@-iana.org", Some(ParsingOptions::new(true))));
    /// assert!(!EmailAddress::is_valid("test", Some(ParsingOptions::new(true))));
    /// assert!(!EmailAddress::is_valid("test", Some(ParsingOptions::new(true))));
    /// ```
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen(js_name = "isValid"))]
    pub fn is_valid(input: &str, options: Option<ParsingOptions>) -> bool {
        EmailAddress::parse_core(input, options).is_some()
    }

    /// Returns the local part of the email address.
    ///
    /// Note that if you are using this library from rust, then consider using the `get_local_part` method instead.
    /// This returns a cloned copy of the local part string, instead of a borrowed `&str`, and exists purely for WASM interoperability.
    ///
    /// # Examples
    /// ```
    /// use email_address_parser::EmailAddress;
    ///
    /// let email = EmailAddress::new("foo", "bar.com", None).unwrap();
    /// assert_eq!(email.localPart(), "foo");
    ///
    /// let email = EmailAddress::parse("foo@bar.com", None).unwrap();
    /// assert_eq!(email.localPart(), "foo");
    /// ```
    #[doc(hidden)]
    #[allow(non_snake_case)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen(getter))]
    pub fn localPart(&self) -> String {
        self.local_part.clone()
    }

    /// Returns the domain of the email address.
    ///
    /// Note that if you are using this library from rust, then consider using the `get_domain` method instead.
    /// This returns a cloned copy of the domain string, instead of a borrowed `&str`, and exists purely for WASM interoperability.
    ///
    /// # Examples
    /// ```
    /// use email_address_parser::EmailAddress;
    ///
    /// let email = EmailAddress::new("foo", "bar.com", None).unwrap();
    /// assert_eq!(email.domain(), "bar.com");
    ///
    /// let email = EmailAddress::parse("foo@bar.com", None).unwrap();
    /// assert_eq!(email.domain(), "bar.com");
    /// ```
    #[doc(hidden)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen(getter))]
    pub fn domain(&self) -> String {
        self.domain.clone()
    }

    /// Returns the formatted EmailAddress.
    /// This exists purely for WASM interoperability.
    #[doc(hidden)]
    #[allow(non_snake_case)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen(skip_typescript))]
    pub fn toString(&self) -> String {
        format!("{}@{}", self.local_part, self.domain)
    }

    fn parse_core<'i>(input: &'i str, options: Option<ParsingOptions>) -> Option<Pairs<'i, Rule>> {
        let options = options.unwrap_or_default();
        let is_strict = !options.is_lax;
        match RFC5322::parse(Rule::address_single, input) {
            Ok(parsed) => Some(parsed),
            Err(_) => {
                if is_strict {
                    None
                } else {
                    match RFC5322::parse(Rule::address_single_obs, input) {
                        Ok(parsed) => Some(parsed),
                        Err(_) => None,
                    }
                }
            }
        }
    }
}

impl EmailAddress {
    #![warn(missing_docs)]
    #![warn(rustdoc::missing_doc_code_examples)]

    /// Instantiates a new `Some(EmailAddress)` for a valid local part and domain.
    /// Returns `Err` otherwise.
    ///
    /// # Examples
    /// ```
    /// use email_address_parser::EmailAddress;
    ///
    /// let email = EmailAddress::new("foo", "bar.com", None).unwrap();
    ///
    /// assert_eq!(EmailAddress::new("foo", "-bar.com", None).is_err(), true);
    /// ```
    pub fn new(
        local_part: &str,
        domain: &str,
        options: Option<ParsingOptions>,
    ) -> Result<EmailAddress, String> {
        match EmailAddress::parse(&format!("{}@{}", local_part, domain), options.clone()) {
            Some(email_address) => Ok(email_address),
            None => {
                if !options.unwrap_or_default().is_lax {
                    return Err(format!("Invalid local part '{}'.", local_part));
                }
                Ok(EmailAddress {
                    local_part: String::from(local_part),
                    domain: String::from(domain),
                })
            }
        }
    }

    /// Returns the local part of the email address.
    ///
    /// Not accessible from WASM.
    ///
    /// # Examples
    /// ```
    /// use email_address_parser::EmailAddress;
    ///
    /// let email = EmailAddress::new("foo", "bar.com", None).unwrap();
    /// assert_eq!(email.get_local_part(), "foo");
    ///
    /// let email = EmailAddress::parse("foo@bar.com", None).unwrap();
    /// assert_eq!(email.get_local_part(), "foo");
    /// ```
    pub fn get_local_part(&self) -> &str {
        self.local_part.as_str()
    }
    /// Returns the domain of the email address.
    ///
    /// Not accessible from WASM.
    ///
    /// # Examples
    /// ```
    /// use email_address_parser::EmailAddress;
    ///
    /// let email = EmailAddress::new("foo", "bar.com", None).unwrap();
    /// assert_eq!(email.get_domain(), "bar.com");
    ///
    /// let email = EmailAddress::parse("foo@bar.com", None).unwrap();
    /// assert_eq!(email.get_domain(), "bar.com");
    /// ```
    pub fn get_domain(&self) -> &str {
        self.domain.as_str()
    }
}

impl fmt::Display for EmailAddress {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        formatter.write_fmt(format_args!("{}@{}", self.local_part, self.domain))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn email_address_instantiation_works() {
        let address = EmailAddress::new("foo", "bar.com", None).unwrap();
        assert_eq!(address.get_local_part(), "foo");
        assert_eq!(address.get_domain(), "bar.com");
        assert_eq!(format!("{}", address), "foo@bar.com");
    }

    #[test]
    fn email_address_supports_equality_checking() {
        let foo_at_bar_dot_com = EmailAddress::new("foo", "bar.com", None).unwrap();
        let foo_at_bar_dot_com_2 = EmailAddress::new("foo", "bar.com", None).unwrap();
        let foob_at_ar_dot_com = EmailAddress::new("foob", "ar.com", None).unwrap();

        assert_eq!(foo_at_bar_dot_com, foo_at_bar_dot_com);
        assert_eq!(foo_at_bar_dot_com, foo_at_bar_dot_com_2);
        assert_ne!(foo_at_bar_dot_com, foob_at_ar_dot_com);
        assert_ne!(foo_at_bar_dot_com_2, foob_at_ar_dot_com);
    }

    #[test]
    fn domain_rule_does_not_parse_dash_google_dot_com() {
        let address = RFC5322::parse(Rule::domain_complete, "-google.com");
        println!("{:#?}", address);
        assert_eq!(address.is_err(), true);
    }

    #[test]
    fn domain_rule_does_not_parse_dash_google_dot_com_obs() {
        let address = RFC5322::parse(Rule::domain_obs, "-google.com");
        println!("{:#?}", address);
        assert_eq!(address.is_err(), true);
    }

    #[test]
    fn domain_rule_does_not_parse_dash_google_dash_dot_com() {
        let address = RFC5322::parse(Rule::domain_complete, "-google-.com");
        println!("{:#?}", address);
        assert_eq!(address.is_err(), true);
    }

    #[test]
    fn domain_rule_parses_google_dash_dot_com() {
        let address = RFC5322::parse(Rule::domain_complete, "google-.com");
        println!("{:#?}", address);
        assert_eq!(address.is_err(), true);
    }

    #[test]
    fn domain_complete_punycode_domain() {
        let actual = RFC5322::parse(Rule::domain_complete, "xn--masekowski-d0b.pl");
        println!("{:#?}", actual);
        assert_eq!(actual.is_err(), false);
    }

    #[test]
    fn can_parse_deprecated_local_part() {
        let actual = RFC5322::parse(Rule::local_part_obs, "\"test\".\"test\"");
        println!("{:#?}", actual);
        assert_eq!(actual.is_err(), false);
    }

    #[test]
    fn can_parse_email_with_deprecated_local_part() {
        let actual = RFC5322::parse(Rule::address_single_obs, "\"test\".\"test\"@iana.org");
        println!("{:#?}", actual);
        assert_eq!(actual.is_err(), false);
    }

    #[test]
    fn can_parse_domain_with_space() {
        println!("{:#?}", RFC5322::parse(Rule::domain_obs, " iana .com"));
        let actual = EmailAddress::parse("test@ iana .com", Some(ParsingOptions::new(true)));
        println!("{:#?}", actual);
        assert_eq!(actual.is_some(), true, "test@ iana .com");
    }

    #[test]
    fn can_parse_email_with_cfws_near_at() {
        let email = " test @iana.org";
        let actual = EmailAddress::parse(&email, None);
        println!("{:#?}", actual);
        assert_eq!(format!("{}", actual.unwrap()), email);
    }

    #[test]
    fn can_parse_email_with_crlf() {
        let email = "\u{0d}\u{0a} test@iana.org";
        let actual = EmailAddress::parse(&email, Some(ParsingOptions::new(true)));
        println!("{:#?}", actual);
        assert_eq!(format!("{}", actual.unwrap()), email);
    }

    #[test]
    fn can_parse_local_part_with_space() {
        let actual = RFC5322::parse(Rule::address_single_obs, "test . test@iana.org");
        println!("{:#?}", actual);
        assert_eq!(actual.is_err(), false);
    }

    #[test]
    fn can_parse_domain_with_bel() {
        let actual = RFC5322::parse(Rule::domain_literal, "[RFC-5322-\u{07}-domain-literal]");
        println!("{:#?}", actual);
        assert_eq!(actual.is_err(), false);
    }

    #[test]
    fn can_parse_local_part_with_space_and_quote() {
        let actual = RFC5322::parse(Rule::local_part_complete, "\"test test\"");
        println!("{:#?}", actual);
        assert_eq!(actual.is_err(), false);
    }

    #[test]
    fn can_parse_idn() {
        let actual = RFC5322::parse(Rule::domain_complete, "bücher.com");
        println!("{:#?}", actual);
        assert_eq!(actual.is_err(), false);
    }

    #[test]
    fn parsing_empty_local_part_and_domain() {
        let actual = EmailAddress::parse("@", Some(ParsingOptions::new(true)));
        assert_eq!(actual.is_none(), true, "expected none");
        let actual = EmailAddress::new("", "", Some(ParsingOptions::new(false)));
        assert_eq!(actual.is_err(), true, "expected error");
        let actual = EmailAddress::new("", "", Some(ParsingOptions::new(true)));
        assert_eq!(actual.is_ok(), true, "expected ok");
        let actual = actual.unwrap();
        assert_eq!(actual.domain, "");
        assert_eq!(actual.local_part, "");
    }
}
