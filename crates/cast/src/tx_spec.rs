use eyre::{Result, eyre};

/// Parsed transaction specification
#[derive(Debug, Clone)]
pub struct TxSpec {
    pub to: String,
    pub value: Option<String>,
    pub sig: Option<String>,
    pub args: Vec<String>,
}

impl TxSpec {
    /// Parse transaction spec in format: to\\[:value\\]\\[:sig\\[:args\\]\\]
    pub fn parse(spec: &str) -> Result<Self> {
        if spec.is_empty() {
            return Err(eyre!("Empty transaction specification"));
        }

        let parts: Vec<&str> = spec.split(':').collect();

        let to = parts[0].to_string();
        if to.is_empty() {
            return Err(eyre!("Missing destination address"));
        }

        let mut value = None;
        let mut sig = None;
        let mut args = Vec::new();

        match parts.len() {
            1 => {
                // Just address: "0x123"
            }
            2 => {
                // Address + value OR raw data: "0x123:0.1ether" or "0x123:0x123abc"
                let second = parts[1];
                if second.starts_with("0x") && second.len() > 10 {
                    // Looks like raw data
                    sig = Some(second.to_string());
                } else if !second.is_empty() {
                    // Looks like value
                    value = Some(second.to_string());
                }
            }
            3 => {
                // Address + value + sig: "0x123:0.1ether:transfer(address,uint256)"
                // OR Address + empty + sig: "0x123::transfer(address,uint256)"
                if !parts[1].is_empty() {
                    value = Some(parts[1].to_string());
                }
                if !parts[2].is_empty() {
                    sig = Some(parts[2].to_string());
                }
            }
            4 => {
                // Address + value + sig + args:
                // "0x123:0.1ether:transfer(address,uint256):0x789,1000"
                if !parts[1].is_empty() {
                    value = Some(parts[1].to_string());
                }
                if !parts[2].is_empty() {
                    sig = Some(parts[2].to_string());
                }
                if !parts[3].is_empty() {
                    args = parts[3].split(',').map(|s| s.trim().to_string()).collect();
                }
            }
            _ => {
                return Err(eyre!(
                    "Invalid transaction specification format. Expected: to[:value][:sig[:args]]"
                ));
            }
        }

        Ok(Self { to, value, sig, args })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_address_only() {
        let spec = TxSpec::parse("0x123").unwrap();
        assert_eq!(spec.to, "0x123");
        assert_eq!(spec.value, None);
        assert_eq!(spec.sig, None);
        assert_eq!(spec.args, Vec::<String>::new());
    }

    #[test]
    fn test_parse_with_value() {
        let spec = TxSpec::parse("0x123:1.5ether").unwrap();
        assert_eq!(spec.to, "0x123");
        assert_eq!(spec.value, Some("1.5ether".to_string()));
        assert_eq!(spec.sig, None);
        assert_eq!(spec.args, Vec::<String>::new());
    }

    #[test]
    fn test_parse_with_raw_data() {
        let spec = TxSpec::parse("0x123:0x1234567890abcdef").unwrap();
        assert_eq!(spec.to, "0x123");
        assert_eq!(spec.value, None);
        assert_eq!(spec.sig, Some("0x1234567890abcdef".to_string()));
        assert_eq!(spec.args, Vec::<String>::new());
    }

    #[test]
    fn test_parse_function_call_with_args() {
        let spec = TxSpec::parse("0x123:0.1ether:transfer(address,uint256):0x789,1000").unwrap();
        assert_eq!(spec.to, "0x123");
        assert_eq!(spec.value, Some("0.1ether".to_string()));
        assert_eq!(spec.sig, Some("transfer(address,uint256)".to_string()));
        assert_eq!(spec.args, vec!["0x789".to_string(), "1000".to_string()]);
    }

    #[test]
    fn test_parse_function_call_no_value() {
        let spec = TxSpec::parse("0x123::transfer(address,uint256):0x789,1000").unwrap();
        assert_eq!(spec.to, "0x123");
        assert_eq!(spec.value, None);
        assert_eq!(spec.sig, Some("transfer(address,uint256)".to_string()));
        assert_eq!(spec.args, vec!["0x789".to_string(), "1000".to_string()]);
    }

    #[test]
    fn test_parse_empty_spec() {
        let result = TxSpec::parse("");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Empty transaction specification"));
    }

    #[test]
    fn test_parse_missing_address() {
        let result = TxSpec::parse(":1ether");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Missing destination address"));
    }

    #[test]
    fn test_parse_too_many_parts() {
        let result = TxSpec::parse("0x123:1ether:transfer(address,uint256):0x789,1000:extra");
        assert!(result.is_err());
        assert!(
            result.unwrap_err().to_string().contains("Invalid transaction specification format")
        );
    }
}
