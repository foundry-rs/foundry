use ethers::providers::{Http, Middleware, Provider};

pub fn pad_word(input: &str) -> [u8; 32] {
    let mut word = [0u8; 32];
    let padded_string = format!("{input:0>64}");
    hex::decode_to_slice(padded_string, &mut word).expect("Invalid hex string");
    word
}

pub fn encode_op(opcode: &str, stack_input: Vec<[u8; 32]>) -> String {
    let mut bytes: String = opcode.to_owned();
    for word in stack_input {
        bytes += &hex::encode(word);
    }
    bytes
}

pub async fn get_contract_code(address: &str) -> Vec<u8> {
    let provider = get_provider(None);
    let code = provider.get_code(address, None).await.unwrap();
    code.as_ref().to_owned()
}

pub fn get_provider(rpc_url: Option<&str>) -> Provider<Http> {
    let rpc_url_unwrapped;
    let env_rpc_url = std::env::var("RPC_URL");
    if let Some(url) = rpc_url {
        rpc_url_unwrapped = url;
    } else if let Ok(ref url) = env_rpc_url {
        rpc_url_unwrapped = url;
    } else {
        panic!("No RPC URL provided");
    };
    Provider::<Http>::try_from(rpc_url_unwrapped).expect("could not instantiate HTTP Provider")
}
