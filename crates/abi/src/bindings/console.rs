pub use console::*;
/// This module was auto-generated with ethers-rs Abigen.
/// More information at: <https://github.com/gakonst/ethers-rs>
#[allow(
    clippy::enum_variant_names,
    clippy::too_many_arguments,
    clippy::upper_case_acronyms,
    clippy::type_complexity,
    dead_code,
    non_camel_case_types,
)]
pub mod console {
    #[allow(deprecated)]
    fn __abi() -> ::ethers_core::abi::Abi {
        ::ethers_core::abi::ethabi::Contract {
            constructor: ::core::option::Option::None,
            functions: ::std::collections::BTreeMap::new(),
            events: ::core::convert::From::from([
                (
                    ::std::borrow::ToOwned::to_owned("log"),
                    ::std::vec![
                        ::ethers_core::abi::ethabi::Event {
                            name: ::std::borrow::ToOwned::to_owned("log"),
                            inputs: ::std::vec![
                                ::ethers_core::abi::ethabi::EventParam {
                                    name: ::std::string::String::new(),
                                    kind: ::ethers_core::abi::ethabi::ParamType::String,
                                    indexed: false,
                                },
                            ],
                            anonymous: false,
                        },
                    ],
                ),
                (
                    ::std::borrow::ToOwned::to_owned("log_address"),
                    ::std::vec![
                        ::ethers_core::abi::ethabi::Event {
                            name: ::std::borrow::ToOwned::to_owned("log_address"),
                            inputs: ::std::vec![
                                ::ethers_core::abi::ethabi::EventParam {
                                    name: ::std::string::String::new(),
                                    kind: ::ethers_core::abi::ethabi::ParamType::Address,
                                    indexed: false,
                                },
                            ],
                            anonymous: false,
                        },
                    ],
                ),
                (
                    ::std::borrow::ToOwned::to_owned("log_array"),
                    ::std::vec![
                        ::ethers_core::abi::ethabi::Event {
                            name: ::std::borrow::ToOwned::to_owned("log_array"),
                            inputs: ::std::vec![
                                ::ethers_core::abi::ethabi::EventParam {
                                    name: ::std::borrow::ToOwned::to_owned("val"),
                                    kind: ::ethers_core::abi::ethabi::ParamType::Array(
                                        ::std::boxed::Box::new(
                                            ::ethers_core::abi::ethabi::ParamType::Uint(256usize),
                                        ),
                                    ),
                                    indexed: false,
                                },
                            ],
                            anonymous: false,
                        },
                        ::ethers_core::abi::ethabi::Event {
                            name: ::std::borrow::ToOwned::to_owned("log_array"),
                            inputs: ::std::vec![
                                ::ethers_core::abi::ethabi::EventParam {
                                    name: ::std::borrow::ToOwned::to_owned("val"),
                                    kind: ::ethers_core::abi::ethabi::ParamType::Array(
                                        ::std::boxed::Box::new(
                                            ::ethers_core::abi::ethabi::ParamType::Int(256usize),
                                        ),
                                    ),
                                    indexed: false,
                                },
                            ],
                            anonymous: false,
                        },
                        ::ethers_core::abi::ethabi::Event {
                            name: ::std::borrow::ToOwned::to_owned("log_array"),
                            inputs: ::std::vec![
                                ::ethers_core::abi::ethabi::EventParam {
                                    name: ::std::borrow::ToOwned::to_owned("val"),
                                    kind: ::ethers_core::abi::ethabi::ParamType::Array(
                                        ::std::boxed::Box::new(
                                            ::ethers_core::abi::ethabi::ParamType::Address,
                                        ),
                                    ),
                                    indexed: false,
                                },
                            ],
                            anonymous: false,
                        },
                    ],
                ),
                (
                    ::std::borrow::ToOwned::to_owned("log_bytes"),
                    ::std::vec![
                        ::ethers_core::abi::ethabi::Event {
                            name: ::std::borrow::ToOwned::to_owned("log_bytes"),
                            inputs: ::std::vec![
                                ::ethers_core::abi::ethabi::EventParam {
                                    name: ::std::string::String::new(),
                                    kind: ::ethers_core::abi::ethabi::ParamType::Bytes,
                                    indexed: false,
                                },
                            ],
                            anonymous: false,
                        },
                    ],
                ),
                (
                    ::std::borrow::ToOwned::to_owned("log_bytes32"),
                    ::std::vec![
                        ::ethers_core::abi::ethabi::Event {
                            name: ::std::borrow::ToOwned::to_owned("log_bytes32"),
                            inputs: ::std::vec![
                                ::ethers_core::abi::ethabi::EventParam {
                                    name: ::std::string::String::new(),
                                    kind: ::ethers_core::abi::ethabi::ParamType::FixedBytes(
                                        32usize,
                                    ),
                                    indexed: false,
                                },
                            ],
                            anonymous: false,
                        },
                    ],
                ),
                (
                    ::std::borrow::ToOwned::to_owned("log_int"),
                    ::std::vec![
                        ::ethers_core::abi::ethabi::Event {
                            name: ::std::borrow::ToOwned::to_owned("log_int"),
                            inputs: ::std::vec![
                                ::ethers_core::abi::ethabi::EventParam {
                                    name: ::std::string::String::new(),
                                    kind: ::ethers_core::abi::ethabi::ParamType::Int(256usize),
                                    indexed: false,
                                },
                            ],
                            anonymous: false,
                        },
                    ],
                ),
                (
                    ::std::borrow::ToOwned::to_owned("log_named_address"),
                    ::std::vec![
                        ::ethers_core::abi::ethabi::Event {
                            name: ::std::borrow::ToOwned::to_owned("log_named_address"),
                            inputs: ::std::vec![
                                ::ethers_core::abi::ethabi::EventParam {
                                    name: ::std::borrow::ToOwned::to_owned("key"),
                                    kind: ::ethers_core::abi::ethabi::ParamType::String,
                                    indexed: false,
                                },
                                ::ethers_core::abi::ethabi::EventParam {
                                    name: ::std::borrow::ToOwned::to_owned("val"),
                                    kind: ::ethers_core::abi::ethabi::ParamType::Address,
                                    indexed: false,
                                },
                            ],
                            anonymous: false,
                        },
                    ],
                ),
                (
                    ::std::borrow::ToOwned::to_owned("log_named_array"),
                    ::std::vec![
                        ::ethers_core::abi::ethabi::Event {
                            name: ::std::borrow::ToOwned::to_owned("log_named_array"),
                            inputs: ::std::vec![
                                ::ethers_core::abi::ethabi::EventParam {
                                    name: ::std::borrow::ToOwned::to_owned("key"),
                                    kind: ::ethers_core::abi::ethabi::ParamType::String,
                                    indexed: false,
                                },
                                ::ethers_core::abi::ethabi::EventParam {
                                    name: ::std::borrow::ToOwned::to_owned("val"),
                                    kind: ::ethers_core::abi::ethabi::ParamType::Array(
                                        ::std::boxed::Box::new(
                                            ::ethers_core::abi::ethabi::ParamType::Uint(256usize),
                                        ),
                                    ),
                                    indexed: false,
                                },
                            ],
                            anonymous: false,
                        },
                        ::ethers_core::abi::ethabi::Event {
                            name: ::std::borrow::ToOwned::to_owned("log_named_array"),
                            inputs: ::std::vec![
                                ::ethers_core::abi::ethabi::EventParam {
                                    name: ::std::borrow::ToOwned::to_owned("key"),
                                    kind: ::ethers_core::abi::ethabi::ParamType::String,
                                    indexed: false,
                                },
                                ::ethers_core::abi::ethabi::EventParam {
                                    name: ::std::borrow::ToOwned::to_owned("val"),
                                    kind: ::ethers_core::abi::ethabi::ParamType::Array(
                                        ::std::boxed::Box::new(
                                            ::ethers_core::abi::ethabi::ParamType::Int(256usize),
                                        ),
                                    ),
                                    indexed: false,
                                },
                            ],
                            anonymous: false,
                        },
                        ::ethers_core::abi::ethabi::Event {
                            name: ::std::borrow::ToOwned::to_owned("log_named_array"),
                            inputs: ::std::vec![
                                ::ethers_core::abi::ethabi::EventParam {
                                    name: ::std::borrow::ToOwned::to_owned("key"),
                                    kind: ::ethers_core::abi::ethabi::ParamType::String,
                                    indexed: false,
                                },
                                ::ethers_core::abi::ethabi::EventParam {
                                    name: ::std::borrow::ToOwned::to_owned("val"),
                                    kind: ::ethers_core::abi::ethabi::ParamType::Array(
                                        ::std::boxed::Box::new(
                                            ::ethers_core::abi::ethabi::ParamType::Address,
                                        ),
                                    ),
                                    indexed: false,
                                },
                            ],
                            anonymous: false,
                        },
                    ],
                ),
                (
                    ::std::borrow::ToOwned::to_owned("log_named_bytes"),
                    ::std::vec![
                        ::ethers_core::abi::ethabi::Event {
                            name: ::std::borrow::ToOwned::to_owned("log_named_bytes"),
                            inputs: ::std::vec![
                                ::ethers_core::abi::ethabi::EventParam {
                                    name: ::std::borrow::ToOwned::to_owned("key"),
                                    kind: ::ethers_core::abi::ethabi::ParamType::String,
                                    indexed: false,
                                },
                                ::ethers_core::abi::ethabi::EventParam {
                                    name: ::std::borrow::ToOwned::to_owned("val"),
                                    kind: ::ethers_core::abi::ethabi::ParamType::Bytes,
                                    indexed: false,
                                },
                            ],
                            anonymous: false,
                        },
                    ],
                ),
                (
                    ::std::borrow::ToOwned::to_owned("log_named_bytes32"),
                    ::std::vec![
                        ::ethers_core::abi::ethabi::Event {
                            name: ::std::borrow::ToOwned::to_owned("log_named_bytes32"),
                            inputs: ::std::vec![
                                ::ethers_core::abi::ethabi::EventParam {
                                    name: ::std::borrow::ToOwned::to_owned("key"),
                                    kind: ::ethers_core::abi::ethabi::ParamType::String,
                                    indexed: false,
                                },
                                ::ethers_core::abi::ethabi::EventParam {
                                    name: ::std::borrow::ToOwned::to_owned("val"),
                                    kind: ::ethers_core::abi::ethabi::ParamType::FixedBytes(
                                        32usize,
                                    ),
                                    indexed: false,
                                },
                            ],
                            anonymous: false,
                        },
                    ],
                ),
                (
                    ::std::borrow::ToOwned::to_owned("log_named_decimal_int"),
                    ::std::vec![
                        ::ethers_core::abi::ethabi::Event {
                            name: ::std::borrow::ToOwned::to_owned(
                                "log_named_decimal_int",
                            ),
                            inputs: ::std::vec![
                                ::ethers_core::abi::ethabi::EventParam {
                                    name: ::std::borrow::ToOwned::to_owned("key"),
                                    kind: ::ethers_core::abi::ethabi::ParamType::String,
                                    indexed: false,
                                },
                                ::ethers_core::abi::ethabi::EventParam {
                                    name: ::std::borrow::ToOwned::to_owned("val"),
                                    kind: ::ethers_core::abi::ethabi::ParamType::Int(256usize),
                                    indexed: false,
                                },
                                ::ethers_core::abi::ethabi::EventParam {
                                    name: ::std::borrow::ToOwned::to_owned("decimals"),
                                    kind: ::ethers_core::abi::ethabi::ParamType::Uint(256usize),
                                    indexed: false,
                                },
                            ],
                            anonymous: false,
                        },
                    ],
                ),
                (
                    ::std::borrow::ToOwned::to_owned("log_named_decimal_uint"),
                    ::std::vec![
                        ::ethers_core::abi::ethabi::Event {
                            name: ::std::borrow::ToOwned::to_owned(
                                "log_named_decimal_uint",
                            ),
                            inputs: ::std::vec![
                                ::ethers_core::abi::ethabi::EventParam {
                                    name: ::std::borrow::ToOwned::to_owned("key"),
                                    kind: ::ethers_core::abi::ethabi::ParamType::String,
                                    indexed: false,
                                },
                                ::ethers_core::abi::ethabi::EventParam {
                                    name: ::std::borrow::ToOwned::to_owned("val"),
                                    kind: ::ethers_core::abi::ethabi::ParamType::Uint(256usize),
                                    indexed: false,
                                },
                                ::ethers_core::abi::ethabi::EventParam {
                                    name: ::std::borrow::ToOwned::to_owned("decimals"),
                                    kind: ::ethers_core::abi::ethabi::ParamType::Uint(256usize),
                                    indexed: false,
                                },
                            ],
                            anonymous: false,
                        },
                    ],
                ),
                (
                    ::std::borrow::ToOwned::to_owned("log_named_int"),
                    ::std::vec![
                        ::ethers_core::abi::ethabi::Event {
                            name: ::std::borrow::ToOwned::to_owned("log_named_int"),
                            inputs: ::std::vec![
                                ::ethers_core::abi::ethabi::EventParam {
                                    name: ::std::borrow::ToOwned::to_owned("key"),
                                    kind: ::ethers_core::abi::ethabi::ParamType::String,
                                    indexed: false,
                                },
                                ::ethers_core::abi::ethabi::EventParam {
                                    name: ::std::borrow::ToOwned::to_owned("val"),
                                    kind: ::ethers_core::abi::ethabi::ParamType::Int(256usize),
                                    indexed: false,
                                },
                            ],
                            anonymous: false,
                        },
                    ],
                ),
                (
                    ::std::borrow::ToOwned::to_owned("log_named_string"),
                    ::std::vec![
                        ::ethers_core::abi::ethabi::Event {
                            name: ::std::borrow::ToOwned::to_owned("log_named_string"),
                            inputs: ::std::vec![
                                ::ethers_core::abi::ethabi::EventParam {
                                    name: ::std::borrow::ToOwned::to_owned("key"),
                                    kind: ::ethers_core::abi::ethabi::ParamType::String,
                                    indexed: false,
                                },
                                ::ethers_core::abi::ethabi::EventParam {
                                    name: ::std::borrow::ToOwned::to_owned("val"),
                                    kind: ::ethers_core::abi::ethabi::ParamType::String,
                                    indexed: false,
                                },
                            ],
                            anonymous: false,
                        },
                    ],
                ),
                (
                    ::std::borrow::ToOwned::to_owned("log_named_uint"),
                    ::std::vec![
                        ::ethers_core::abi::ethabi::Event {
                            name: ::std::borrow::ToOwned::to_owned("log_named_uint"),
                            inputs: ::std::vec![
                                ::ethers_core::abi::ethabi::EventParam {
                                    name: ::std::borrow::ToOwned::to_owned("key"),
                                    kind: ::ethers_core::abi::ethabi::ParamType::String,
                                    indexed: false,
                                },
                                ::ethers_core::abi::ethabi::EventParam {
                                    name: ::std::borrow::ToOwned::to_owned("val"),
                                    kind: ::ethers_core::abi::ethabi::ParamType::Uint(256usize),
                                    indexed: false,
                                },
                            ],
                            anonymous: false,
                        },
                    ],
                ),
                (
                    ::std::borrow::ToOwned::to_owned("log_string"),
                    ::std::vec![
                        ::ethers_core::abi::ethabi::Event {
                            name: ::std::borrow::ToOwned::to_owned("log_string"),
                            inputs: ::std::vec![
                                ::ethers_core::abi::ethabi::EventParam {
                                    name: ::std::string::String::new(),
                                    kind: ::ethers_core::abi::ethabi::ParamType::String,
                                    indexed: false,
                                },
                            ],
                            anonymous: false,
                        },
                    ],
                ),
                (
                    ::std::borrow::ToOwned::to_owned("log_uint"),
                    ::std::vec![
                        ::ethers_core::abi::ethabi::Event {
                            name: ::std::borrow::ToOwned::to_owned("log_uint"),
                            inputs: ::std::vec![
                                ::ethers_core::abi::ethabi::EventParam {
                                    name: ::std::string::String::new(),
                                    kind: ::ethers_core::abi::ethabi::ParamType::Uint(256usize),
                                    indexed: false,
                                },
                            ],
                            anonymous: false,
                        },
                    ],
                ),
                (
                    ::std::borrow::ToOwned::to_owned("logs"),
                    ::std::vec![
                        ::ethers_core::abi::ethabi::Event {
                            name: ::std::borrow::ToOwned::to_owned("logs"),
                            inputs: ::std::vec![
                                ::ethers_core::abi::ethabi::EventParam {
                                    name: ::std::string::String::new(),
                                    kind: ::ethers_core::abi::ethabi::ParamType::Bytes,
                                    indexed: false,
                                },
                            ],
                            anonymous: false,
                        },
                    ],
                ),
            ]),
            errors: ::std::collections::BTreeMap::new(),
            receive: false,
            fallback: false,
        }
    }
    ///The parsed human-readable ABI of the contract.
    pub static CONSOLE_ABI: ::ethers_contract::Lazy<::ethers_core::abi::Abi> = ::ethers_contract::Lazy::new(
        __abi,
    );
    #[derive(
        Clone,
        ::ethers_contract::EthEvent,
        ::ethers_contract::EthDisplay,
        Default,
        Debug,
        PartialEq,
        Eq,
        Hash
    )]
    #[ethevent(name = "log", abi = "log(string)")]
    pub struct LogFilter(pub ::std::string::String);
    #[derive(
        Clone,
        ::ethers_contract::EthEvent,
        ::ethers_contract::EthDisplay,
        Default,
        Debug,
        PartialEq,
        Eq,
        Hash
    )]
    #[ethevent(name = "log_address", abi = "log_address(address)")]
    pub struct LogAddressFilter(pub ::ethers_core::types::Address);
    #[derive(
        Clone,
        ::ethers_contract::EthEvent,
        ::ethers_contract::EthDisplay,
        Default,
        Debug,
        PartialEq,
        Eq,
        Hash
    )]
    #[ethevent(name = "log_array", abi = "log_array(uint256[])")]
    pub struct LogArray1Filter {
        pub val: ::std::vec::Vec<::ethers_core::types::U256>,
    }
    #[derive(
        Clone,
        ::ethers_contract::EthEvent,
        ::ethers_contract::EthDisplay,
        Default,
        Debug,
        PartialEq,
        Eq,
        Hash
    )]
    #[ethevent(name = "log_array", abi = "log_array(int256[])")]
    pub struct LogArray2Filter {
        pub val: ::std::vec::Vec<::ethers_core::types::I256>,
    }
    #[derive(
        Clone,
        ::ethers_contract::EthEvent,
        ::ethers_contract::EthDisplay,
        Default,
        Debug,
        PartialEq,
        Eq,
        Hash
    )]
    #[ethevent(name = "log_array", abi = "log_array(address[])")]
    pub struct LogArray3Filter {
        pub val: ::std::vec::Vec<::ethers_core::types::Address>,
    }
    #[derive(
        Clone,
        ::ethers_contract::EthEvent,
        ::ethers_contract::EthDisplay,
        Default,
        Debug,
        PartialEq,
        Eq,
        Hash
    )]
    #[ethevent(name = "log_bytes", abi = "log_bytes(bytes)")]
    pub struct LogBytesFilter(pub ::ethers_core::types::Bytes);
    #[derive(
        Clone,
        ::ethers_contract::EthEvent,
        ::ethers_contract::EthDisplay,
        Default,
        Debug,
        PartialEq,
        Eq,
        Hash
    )]
    #[ethevent(name = "log_bytes32", abi = "log_bytes32(bytes32)")]
    pub struct LogBytes32Filter(pub [u8; 32]);
    #[derive(
        Clone,
        ::ethers_contract::EthEvent,
        ::ethers_contract::EthDisplay,
        Default,
        Debug,
        PartialEq,
        Eq,
        Hash
    )]
    #[ethevent(name = "log_int", abi = "log_int(int256)")]
    pub struct LogIntFilter(pub ::ethers_core::types::I256);
    #[derive(
        Clone,
        ::ethers_contract::EthEvent,
        ::ethers_contract::EthDisplay,
        Default,
        Debug,
        PartialEq,
        Eq,
        Hash
    )]
    #[ethevent(name = "log_named_address", abi = "log_named_address(string,address)")]
    pub struct LogNamedAddressFilter {
        pub key: ::std::string::String,
        pub val: ::ethers_core::types::Address,
    }
    #[derive(
        Clone,
        ::ethers_contract::EthEvent,
        ::ethers_contract::EthDisplay,
        Default,
        Debug,
        PartialEq,
        Eq,
        Hash
    )]
    #[ethevent(name = "log_named_array", abi = "log_named_array(string,uint256[])")]
    pub struct LogNamedArray1Filter {
        pub key: ::std::string::String,
        pub val: ::std::vec::Vec<::ethers_core::types::U256>,
    }
    #[derive(
        Clone,
        ::ethers_contract::EthEvent,
        ::ethers_contract::EthDisplay,
        Default,
        Debug,
        PartialEq,
        Eq,
        Hash
    )]
    #[ethevent(name = "log_named_array", abi = "log_named_array(string,int256[])")]
    pub struct LogNamedArray2Filter {
        pub key: ::std::string::String,
        pub val: ::std::vec::Vec<::ethers_core::types::I256>,
    }
    #[derive(
        Clone,
        ::ethers_contract::EthEvent,
        ::ethers_contract::EthDisplay,
        Default,
        Debug,
        PartialEq,
        Eq,
        Hash
    )]
    #[ethevent(name = "log_named_array", abi = "log_named_array(string,address[])")]
    pub struct LogNamedArray3Filter {
        pub key: ::std::string::String,
        pub val: ::std::vec::Vec<::ethers_core::types::Address>,
    }
    #[derive(
        Clone,
        ::ethers_contract::EthEvent,
        ::ethers_contract::EthDisplay,
        Default,
        Debug,
        PartialEq,
        Eq,
        Hash
    )]
    #[ethevent(name = "log_named_bytes", abi = "log_named_bytes(string,bytes)")]
    pub struct LogNamedBytesFilter {
        pub key: ::std::string::String,
        pub val: ::ethers_core::types::Bytes,
    }
    #[derive(
        Clone,
        ::ethers_contract::EthEvent,
        ::ethers_contract::EthDisplay,
        Default,
        Debug,
        PartialEq,
        Eq,
        Hash
    )]
    #[ethevent(name = "log_named_bytes32", abi = "log_named_bytes32(string,bytes32)")]
    pub struct LogNamedBytes32Filter {
        pub key: ::std::string::String,
        pub val: [u8; 32],
    }
    #[derive(
        Clone,
        ::ethers_contract::EthEvent,
        ::ethers_contract::EthDisplay,
        Default,
        Debug,
        PartialEq,
        Eq,
        Hash
    )]
    #[ethevent(
        name = "log_named_decimal_int",
        abi = "log_named_decimal_int(string,int256,uint256)"
    )]
    pub struct LogNamedDecimalIntFilter {
        pub key: ::std::string::String,
        pub val: ::ethers_core::types::I256,
        pub decimals: ::ethers_core::types::U256,
    }
    #[derive(
        Clone,
        ::ethers_contract::EthEvent,
        ::ethers_contract::EthDisplay,
        Default,
        Debug,
        PartialEq,
        Eq,
        Hash
    )]
    #[ethevent(
        name = "log_named_decimal_uint",
        abi = "log_named_decimal_uint(string,uint256,uint256)"
    )]
    pub struct LogNamedDecimalUintFilter {
        pub key: ::std::string::String,
        pub val: ::ethers_core::types::U256,
        pub decimals: ::ethers_core::types::U256,
    }
    #[derive(
        Clone,
        ::ethers_contract::EthEvent,
        ::ethers_contract::EthDisplay,
        Default,
        Debug,
        PartialEq,
        Eq,
        Hash
    )]
    #[ethevent(name = "log_named_int", abi = "log_named_int(string,int256)")]
    pub struct LogNamedIntFilter {
        pub key: ::std::string::String,
        pub val: ::ethers_core::types::I256,
    }
    #[derive(
        Clone,
        ::ethers_contract::EthEvent,
        ::ethers_contract::EthDisplay,
        Default,
        Debug,
        PartialEq,
        Eq,
        Hash
    )]
    #[ethevent(name = "log_named_string", abi = "log_named_string(string,string)")]
    pub struct LogNamedStringFilter {
        pub key: ::std::string::String,
        pub val: ::std::string::String,
    }
    #[derive(
        Clone,
        ::ethers_contract::EthEvent,
        ::ethers_contract::EthDisplay,
        Default,
        Debug,
        PartialEq,
        Eq,
        Hash
    )]
    #[ethevent(name = "log_named_uint", abi = "log_named_uint(string,uint256)")]
    pub struct LogNamedUintFilter {
        pub key: ::std::string::String,
        pub val: ::ethers_core::types::U256,
    }
    #[derive(
        Clone,
        ::ethers_contract::EthEvent,
        ::ethers_contract::EthDisplay,
        Default,
        Debug,
        PartialEq,
        Eq,
        Hash
    )]
    #[ethevent(name = "log_string", abi = "log_string(string)")]
    pub struct LogStringFilter(pub ::std::string::String);
    #[derive(
        Clone,
        ::ethers_contract::EthEvent,
        ::ethers_contract::EthDisplay,
        Default,
        Debug,
        PartialEq,
        Eq,
        Hash
    )]
    #[ethevent(name = "log_uint", abi = "log_uint(uint256)")]
    pub struct LogUintFilter(pub ::ethers_core::types::U256);
    #[derive(
        Clone,
        ::ethers_contract::EthEvent,
        ::ethers_contract::EthDisplay,
        Default,
        Debug,
        PartialEq,
        Eq,
        Hash
    )]
    #[ethevent(name = "logs", abi = "logs(bytes)")]
    pub struct LogsFilter(pub ::ethers_core::types::Bytes);
    ///Container type for all of the contract's events
    #[derive(Clone, ::ethers_contract::EthAbiType, Debug, PartialEq, Eq, Hash)]
    pub enum ConsoleEvents {
        LogFilter(LogFilter),
        LogAddressFilter(LogAddressFilter),
        LogArray1Filter(LogArray1Filter),
        LogArray2Filter(LogArray2Filter),
        LogArray3Filter(LogArray3Filter),
        LogBytesFilter(LogBytesFilter),
        LogBytes32Filter(LogBytes32Filter),
        LogIntFilter(LogIntFilter),
        LogNamedAddressFilter(LogNamedAddressFilter),
        LogNamedArray1Filter(LogNamedArray1Filter),
        LogNamedArray2Filter(LogNamedArray2Filter),
        LogNamedArray3Filter(LogNamedArray3Filter),
        LogNamedBytesFilter(LogNamedBytesFilter),
        LogNamedBytes32Filter(LogNamedBytes32Filter),
        LogNamedDecimalIntFilter(LogNamedDecimalIntFilter),
        LogNamedDecimalUintFilter(LogNamedDecimalUintFilter),
        LogNamedIntFilter(LogNamedIntFilter),
        LogNamedStringFilter(LogNamedStringFilter),
        LogNamedUintFilter(LogNamedUintFilter),
        LogStringFilter(LogStringFilter),
        LogUintFilter(LogUintFilter),
        LogsFilter(LogsFilter),
    }
    impl ::ethers_contract::EthLogDecode for ConsoleEvents {
        fn decode_log(
            log: &::ethers_core::abi::RawLog,
        ) -> ::core::result::Result<Self, ::ethers_core::abi::Error> {
            if let Ok(decoded) = LogFilter::decode_log(log) {
                return Ok(ConsoleEvents::LogFilter(decoded));
            }
            if let Ok(decoded) = LogAddressFilter::decode_log(log) {
                return Ok(ConsoleEvents::LogAddressFilter(decoded));
            }
            if let Ok(decoded) = LogArray1Filter::decode_log(log) {
                return Ok(ConsoleEvents::LogArray1Filter(decoded));
            }
            if let Ok(decoded) = LogArray2Filter::decode_log(log) {
                return Ok(ConsoleEvents::LogArray2Filter(decoded));
            }
            if let Ok(decoded) = LogArray3Filter::decode_log(log) {
                return Ok(ConsoleEvents::LogArray3Filter(decoded));
            }
            if let Ok(decoded) = LogBytesFilter::decode_log(log) {
                return Ok(ConsoleEvents::LogBytesFilter(decoded));
            }
            if let Ok(decoded) = LogBytes32Filter::decode_log(log) {
                return Ok(ConsoleEvents::LogBytes32Filter(decoded));
            }
            if let Ok(decoded) = LogIntFilter::decode_log(log) {
                return Ok(ConsoleEvents::LogIntFilter(decoded));
            }
            if let Ok(decoded) = LogNamedAddressFilter::decode_log(log) {
                return Ok(ConsoleEvents::LogNamedAddressFilter(decoded));
            }
            if let Ok(decoded) = LogNamedArray1Filter::decode_log(log) {
                return Ok(ConsoleEvents::LogNamedArray1Filter(decoded));
            }
            if let Ok(decoded) = LogNamedArray2Filter::decode_log(log) {
                return Ok(ConsoleEvents::LogNamedArray2Filter(decoded));
            }
            if let Ok(decoded) = LogNamedArray3Filter::decode_log(log) {
                return Ok(ConsoleEvents::LogNamedArray3Filter(decoded));
            }
            if let Ok(decoded) = LogNamedBytesFilter::decode_log(log) {
                return Ok(ConsoleEvents::LogNamedBytesFilter(decoded));
            }
            if let Ok(decoded) = LogNamedBytes32Filter::decode_log(log) {
                return Ok(ConsoleEvents::LogNamedBytes32Filter(decoded));
            }
            if let Ok(decoded) = LogNamedDecimalIntFilter::decode_log(log) {
                return Ok(ConsoleEvents::LogNamedDecimalIntFilter(decoded));
            }
            if let Ok(decoded) = LogNamedDecimalUintFilter::decode_log(log) {
                return Ok(ConsoleEvents::LogNamedDecimalUintFilter(decoded));
            }
            if let Ok(decoded) = LogNamedIntFilter::decode_log(log) {
                return Ok(ConsoleEvents::LogNamedIntFilter(decoded));
            }
            if let Ok(decoded) = LogNamedStringFilter::decode_log(log) {
                return Ok(ConsoleEvents::LogNamedStringFilter(decoded));
            }
            if let Ok(decoded) = LogNamedUintFilter::decode_log(log) {
                return Ok(ConsoleEvents::LogNamedUintFilter(decoded));
            }
            if let Ok(decoded) = LogStringFilter::decode_log(log) {
                return Ok(ConsoleEvents::LogStringFilter(decoded));
            }
            if let Ok(decoded) = LogUintFilter::decode_log(log) {
                return Ok(ConsoleEvents::LogUintFilter(decoded));
            }
            if let Ok(decoded) = LogsFilter::decode_log(log) {
                return Ok(ConsoleEvents::LogsFilter(decoded));
            }
            Err(::ethers_core::abi::Error::InvalidData)
        }
    }
    impl ::core::fmt::Display for ConsoleEvents {
        fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
            match self {
                Self::LogFilter(element) => ::core::fmt::Display::fmt(element, f),
                Self::LogAddressFilter(element) => ::core::fmt::Display::fmt(element, f),
                Self::LogArray1Filter(element) => ::core::fmt::Display::fmt(element, f),
                Self::LogArray2Filter(element) => ::core::fmt::Display::fmt(element, f),
                Self::LogArray3Filter(element) => ::core::fmt::Display::fmt(element, f),
                Self::LogBytesFilter(element) => ::core::fmt::Display::fmt(element, f),
                Self::LogBytes32Filter(element) => ::core::fmt::Display::fmt(element, f),
                Self::LogIntFilter(element) => ::core::fmt::Display::fmt(element, f),
                Self::LogNamedAddressFilter(element) => {
                    ::core::fmt::Display::fmt(element, f)
                }
                Self::LogNamedArray1Filter(element) => {
                    ::core::fmt::Display::fmt(element, f)
                }
                Self::LogNamedArray2Filter(element) => {
                    ::core::fmt::Display::fmt(element, f)
                }
                Self::LogNamedArray3Filter(element) => {
                    ::core::fmt::Display::fmt(element, f)
                }
                Self::LogNamedBytesFilter(element) => {
                    ::core::fmt::Display::fmt(element, f)
                }
                Self::LogNamedBytes32Filter(element) => {
                    ::core::fmt::Display::fmt(element, f)
                }
                Self::LogNamedDecimalIntFilter(element) => {
                    ::core::fmt::Display::fmt(element, f)
                }
                Self::LogNamedDecimalUintFilter(element) => {
                    ::core::fmt::Display::fmt(element, f)
                }
                Self::LogNamedIntFilter(element) => ::core::fmt::Display::fmt(element, f),
                Self::LogNamedStringFilter(element) => {
                    ::core::fmt::Display::fmt(element, f)
                }
                Self::LogNamedUintFilter(element) => {
                    ::core::fmt::Display::fmt(element, f)
                }
                Self::LogStringFilter(element) => ::core::fmt::Display::fmt(element, f),
                Self::LogUintFilter(element) => ::core::fmt::Display::fmt(element, f),
                Self::LogsFilter(element) => ::core::fmt::Display::fmt(element, f),
            }
        }
    }
    impl ::core::convert::From<LogFilter> for ConsoleEvents {
        fn from(value: LogFilter) -> Self {
            Self::LogFilter(value)
        }
    }
    impl ::core::convert::From<LogAddressFilter> for ConsoleEvents {
        fn from(value: LogAddressFilter) -> Self {
            Self::LogAddressFilter(value)
        }
    }
    impl ::core::convert::From<LogArray1Filter> for ConsoleEvents {
        fn from(value: LogArray1Filter) -> Self {
            Self::LogArray1Filter(value)
        }
    }
    impl ::core::convert::From<LogArray2Filter> for ConsoleEvents {
        fn from(value: LogArray2Filter) -> Self {
            Self::LogArray2Filter(value)
        }
    }
    impl ::core::convert::From<LogArray3Filter> for ConsoleEvents {
        fn from(value: LogArray3Filter) -> Self {
            Self::LogArray3Filter(value)
        }
    }
    impl ::core::convert::From<LogBytesFilter> for ConsoleEvents {
        fn from(value: LogBytesFilter) -> Self {
            Self::LogBytesFilter(value)
        }
    }
    impl ::core::convert::From<LogBytes32Filter> for ConsoleEvents {
        fn from(value: LogBytes32Filter) -> Self {
            Self::LogBytes32Filter(value)
        }
    }
    impl ::core::convert::From<LogIntFilter> for ConsoleEvents {
        fn from(value: LogIntFilter) -> Self {
            Self::LogIntFilter(value)
        }
    }
    impl ::core::convert::From<LogNamedAddressFilter> for ConsoleEvents {
        fn from(value: LogNamedAddressFilter) -> Self {
            Self::LogNamedAddressFilter(value)
        }
    }
    impl ::core::convert::From<LogNamedArray1Filter> for ConsoleEvents {
        fn from(value: LogNamedArray1Filter) -> Self {
            Self::LogNamedArray1Filter(value)
        }
    }
    impl ::core::convert::From<LogNamedArray2Filter> for ConsoleEvents {
        fn from(value: LogNamedArray2Filter) -> Self {
            Self::LogNamedArray2Filter(value)
        }
    }
    impl ::core::convert::From<LogNamedArray3Filter> for ConsoleEvents {
        fn from(value: LogNamedArray3Filter) -> Self {
            Self::LogNamedArray3Filter(value)
        }
    }
    impl ::core::convert::From<LogNamedBytesFilter> for ConsoleEvents {
        fn from(value: LogNamedBytesFilter) -> Self {
            Self::LogNamedBytesFilter(value)
        }
    }
    impl ::core::convert::From<LogNamedBytes32Filter> for ConsoleEvents {
        fn from(value: LogNamedBytes32Filter) -> Self {
            Self::LogNamedBytes32Filter(value)
        }
    }
    impl ::core::convert::From<LogNamedDecimalIntFilter> for ConsoleEvents {
        fn from(value: LogNamedDecimalIntFilter) -> Self {
            Self::LogNamedDecimalIntFilter(value)
        }
    }
    impl ::core::convert::From<LogNamedDecimalUintFilter> for ConsoleEvents {
        fn from(value: LogNamedDecimalUintFilter) -> Self {
            Self::LogNamedDecimalUintFilter(value)
        }
    }
    impl ::core::convert::From<LogNamedIntFilter> for ConsoleEvents {
        fn from(value: LogNamedIntFilter) -> Self {
            Self::LogNamedIntFilter(value)
        }
    }
    impl ::core::convert::From<LogNamedStringFilter> for ConsoleEvents {
        fn from(value: LogNamedStringFilter) -> Self {
            Self::LogNamedStringFilter(value)
        }
    }
    impl ::core::convert::From<LogNamedUintFilter> for ConsoleEvents {
        fn from(value: LogNamedUintFilter) -> Self {
            Self::LogNamedUintFilter(value)
        }
    }
    impl ::core::convert::From<LogStringFilter> for ConsoleEvents {
        fn from(value: LogStringFilter) -> Self {
            Self::LogStringFilter(value)
        }
    }
    impl ::core::convert::From<LogUintFilter> for ConsoleEvents {
        fn from(value: LogUintFilter) -> Self {
            Self::LogUintFilter(value)
        }
    }
    impl ::core::convert::From<LogsFilter> for ConsoleEvents {
        fn from(value: LogsFilter) -> Self {
            Self::LogsFilter(value)
        }
    }
}
