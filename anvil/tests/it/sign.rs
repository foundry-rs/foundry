use anvil::{spawn, NodeConfig};
use ethers::{
    prelude::{Middleware, SignerMiddleware},
    signers::Signer,
    types::{transaction::eip712::TypedData, Address, Chain, TransactionRequest},
};

#[tokio::test(flavor = "multi_thread")]
async fn can_sign_typed_data() {
    let (api, _handle) = spawn(NodeConfig::test()).await;

    let json = serde_json::json!(
            {
      "types": {
        "EIP712Domain": [
          {
            "name": "name",
            "type": "string"
          },
          {
            "name": "version",
            "type": "string"
          },
          {
            "name": "chainId",
            "type": "uint256"
          },
          {
            "name": "verifyingContract",
            "type": "address"
          }
        ],
        "Person": [
          {
            "name": "name",
            "type": "string"
          },
          {
            "name": "wallet",
            "type": "address"
          }
        ],
        "Mail": [
          {
            "name": "from",
            "type": "Person"
          },
          {
            "name": "to",
            "type": "Person"
          },
          {
            "name": "contents",
            "type": "string"
          }
        ]
      },
      "primaryType": "Mail",
      "domain": {
        "name": "Ether Mail",
        "version": "1",
        "chainId": 1,
        "verifyingContract": "0xCcCCccccCCCCcCCCCCCcCcCccCcCCCcCcccccccC"
      },
      "message": {
        "from": {
          "name": "Cow",
          "wallet": "0xCD2a3d9F938E13CD947Ec05AbC7FE734Df8DD826"
        },
        "to": {
          "name": "Bob",
          "wallet": "0xbBbBBBBbbBBBbbbBbbBbbbbBBbBbbbbBbBbbBBbB"
        },
        "contents": "Hello, Bob!"
      }
    });

    let typed_data: TypedData = serde_json::from_value(json).unwrap();

    // `curl -X POST -H "Content-Type: application/json" -d '{"jsonrpc":"2.0","method": "eth_signTypedData_v4", "params": ["0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266", {"types":{"EIP712Domain":[{"name":"name","type":"string"},{"name":"version","type":"string"},{"name":"chainId","type":"uint256"},{"name":"verifyingContract","type":"address"}],"Person":[{"name":"name","type":"string"},{"name":"wallet","type":"address"}],"Mail":[{"name":"from","type":"Person"},{"name":"to","type":"Person"},{"name":"contents","type":"string"}]},"primaryType":"Mail","domain":{"name":"Ether Mail","version":"1","chainId":1,"verifyingContract":"0xCcCCccccCCCCcCCCCCCcCcCccCcCCCcCcccccccC"},"message":{"from":{"name":"Cow","wallet":"0xCD2a3d9F938E13CD947Ec05AbC7FE734Df8DD826"},"to":{"name":"Bob","wallet":"0xbBbBBBBbbBBBbbbBbbBbbbbBBbBbbbbBbBbbBBbB"},"contents":"Hello, Bob!"}}],"id":67}' http://localhost:8545`

    let signature = api
        .sign_typed_data_v4(
            "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266".parse().unwrap(),
            &typed_data,
        )
        .await
        .unwrap();
    assert_eq!(
      signature,
      "0x6ea8bb309a3401225701f3565e32519f94a0ea91a5910ce9229fe488e773584c0390416a2190d9560219dab757ecca2029e63fa9d1c2aebf676cc25b9f03126a1b".to_string()
    );
}

// <https://github.com/foundry-rs/foundry/issues/2458>
#[tokio::test(flavor = "multi_thread")]
async fn can_sign_typed_data_os() {
    let (api, _handle) = spawn(NodeConfig::test()).await;

    let json = serde_json::json!(
    {
      "types": {
        "EIP712Domain": [
          {
            "name": "name",
            "type": "string"
          },
          {
            "name": "version",
            "type": "string"
          },
          {
            "name": "chainId",
            "type": "uint256"
          },
          {
            "name": "verifyingContract",
            "type": "address"
          }
        ],
        "OrderComponents": [
          {
            "name": "offerer",
            "type": "address"
          },
          {
            "name": "zone",
            "type": "address"
          },
          {
            "name": "offer",
            "type": "OfferItem[]"
          },
          {
            "name": "consideration",
            "type": "ConsiderationItem[]"
          },
          {
            "name": "orderType",
            "type": "uint8"
          },
          {
            "name": "startTime",
            "type": "uint256"
          },
          {
            "name": "endTime",
            "type": "uint256"
          },
          {
            "name": "zoneHash",
            "type": "bytes32"
          },
          {
            "name": "salt",
            "type": "uint256"
          },
          {
            "name": "conduitKey",
            "type": "bytes32"
          },
          {
            "name": "counter",
            "type": "uint256"
          }
        ],
        "OfferItem": [
          {
            "name": "itemType",
            "type": "uint8"
          },
          {
            "name": "token",
            "type": "address"
          },
          {
            "name": "identifierOrCriteria",
            "type": "uint256"
          },
          {
            "name": "startAmount",
            "type": "uint256"
          },
          {
            "name": "endAmount",
            "type": "uint256"
          }
        ],
        "ConsiderationItem": [
          {
            "name": "itemType",
            "type": "uint8"
          },
          {
            "name": "token",
            "type": "address"
          },
          {
            "name": "identifierOrCriteria",
            "type": "uint256"
          },
          {
            "name": "startAmount",
            "type": "uint256"
          },
          {
            "name": "endAmount",
            "type": "uint256"
          },
          {
            "name": "recipient",
            "type": "address"
          }
        ]
      },
      "primaryType": "OrderComponents",
      "domain": {
        "name": "Seaport",
        "version": "1.1",
        "chainId": "1",
        "verifyingContract": "0x00000000006c3852cbEf3e08E8dF289169EdE581"
      },
      "message": {
        "offerer": "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266",
        "offer": [
          {
            "itemType": "3",
            "token": "0xA604060890923Ff400e8c6f5290461A83AEDACec",
            "identifierOrCriteria": "110194434039389003190498847789203126033799499726478230611233094448886344768909",
            "startAmount": "1",
            "endAmount": "1"
          }
        ],
        "consideration": [
          {
            "itemType": "0",
            "token": "0x0000000000000000000000000000000000000000",
            "identifierOrCriteria": "0",
            "startAmount": "487500000000000000",
            "endAmount": "487500000000000000",
            "recipient": "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"
          },
          {
            "itemType": "0",
            "token": "0x0000000000000000000000000000000000000000",
            "identifierOrCriteria": "0",
            "startAmount": "12500000000000000",
            "endAmount": "12500000000000000",
            "recipient": "0x8De9C5A032463C561423387a9648c5C7BCC5BC90"
          }
        ],
        "startTime": "1658645591",
        "endTime": "1659250386",
        "orderType": "3",
        "zone": "0x004C00500000aD104D7DBd00e3ae0A5C00560C00",
        "zoneHash": "0x0000000000000000000000000000000000000000000000000000000000000000",
        "salt": "16178208897136618",
        "conduitKey": "0x0000007b02230091a7ed01230072f7006a004d60a8d4e71d599b8104250f0000",
        "totalOriginalConsiderationItems": "2",
        "counter": "0"
      }
    }
        );

    let typed_data: TypedData = serde_json::from_value(json).unwrap();

    // `curl -X POST http://localhost:8545 -d '{"jsonrpc": "2.0", "method": "eth_signTypedData_v4", "params": ["0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266", {"types":{"EIP712Domain":[{"name":"name","type":"string"},{"name":"version","type":"string"},{"name":"chainId","type":"uint256"},{"name":"verifyingContract","type":"address"}],"OrderComponents":[{"name":"offerer","type":"address"},{"name":"zone","type":"address"},{"name":"offer","type":"OfferItem[]"},{"name":"consideration","type":"ConsiderationItem[]"},{"name":"orderType","type":"uint8"},{"name":"startTime","type":"uint256"},{"name":"endTime","type":"uint256"},{"name":"zoneHash","type":"bytes32"},{"name":"salt","type":"uint256"},{"name":"conduitKey","type":"bytes32"},{"name":"counter","type":"uint256"}],"OfferItem":[{"name":"itemType","type":"uint8"},{"name":"token","type":"address"},{"name":"identifierOrCriteria","type":"uint256"},{"name":"startAmount","type":"uint256"},{"name":"endAmount","type":"uint256"}],"ConsiderationItem":[{"name":"itemType","type":"uint8"},{"name":"token","type":"address"},{"name":"identifierOrCriteria","type":"uint256"},{"name":"startAmount","type":"uint256"},{"name":"endAmount","type":"uint256"},{"name":"recipient","type":"address"}]},"primaryType":"OrderComponents","domain":{"name":"Seaport","version":"1.1","chainId":"1","verifyingContract":"0x00000000006c3852cbEf3e08E8dF289169EdE581"},"message":{"offerer":"0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266","offer":[{"itemType":"3","token":"0xA604060890923Ff400e8c6f5290461A83AEDACec","identifierOrCriteria":"110194434039389003190498847789203126033799499726478230611233094448886344768909","startAmount":"1","endAmount":"1"}],"consideration":[{"itemType":"0","token":"0x0000000000000000000000000000000000000000","identifierOrCriteria":"0","startAmount":"487500000000000000","endAmount":"487500000000000000","recipient":"0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"},{"itemType":"0","token":"0x0000000000000000000000000000000000000000","identifierOrCriteria":"0","startAmount":"12500000000000000","endAmount":"12500000000000000","recipient":"0x8De9C5A032463C561423387a9648c5C7BCC5BC90"}],"startTime":"1658645591","endTime":"1659250386","orderType":"3","zone":"0x004C00500000aD104D7DBd00e3ae0A5C00560C00","zoneHash":"0x0000000000000000000000000000000000000000000000000000000000000000","salt":"16178208897136618","conduitKey":"0x0000007b02230091a7ed01230072f7006a004d60a8d4e71d599b8104250f0000","totalOriginalConsiderationItems":"2","counter":"0"}}], "id": "1"}' -H "Content-Type: application/json"`

    let signature = api
        .sign_typed_data_v4(
            "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266".parse().unwrap(),
            &typed_data,
        )
        .await
        .unwrap();

    assert_eq!(
      signature,
      "0xedb0fa55ac67e3ca52b6bd6ee3576b193731adc2aff42151f67826932fa9f6191261ebdecc2c650204ff7625752b033293fb67ef5cfca78e16de359200040b761b".to_string()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn rejects_different_chain_id() {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();

    let wallet = handle.dev_wallets().next().unwrap();
    let client = SignerMiddleware::new(provider, wallet.with_chain_id(Chain::Mainnet));

    let tx = TransactionRequest::new().to(Address::random()).value(100u64);

    let res = client.send_transaction(tx, None).await;
    let err = res.unwrap_err();
    assert!(err.to_string().contains("signed for another chain"));
}

#[tokio::test]
async fn rejects_invalid_chain_id() {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let wallet = handle.dev_wallets().next().unwrap().with_chain_id(99u64);
    let provider = handle.http_provider();
    let client = SignerMiddleware::new(provider, wallet);
    let tx = TransactionRequest::new().to(Address::random()).value(100u64);
    let res = client.send_transaction(tx, None).await;
    let _err = res.unwrap_err();
}

// <https://github.com/foundry-rs/foundry/issues/3409>
#[tokio::test(flavor = "multi_thread")]
async fn can_sign_typed_seaport_data() {
    let (api, _handle) = spawn(NodeConfig::test()).await;

    let json = serde_json::json!(
       {
      "types": {
        "EIP712Domain": [
          {
            "name": "name",
            "type": "string"
          },
          {
            "name": "version",
            "type": "string"
          },
          {
            "name": "chainId",
            "type": "uint256"
          },
          {
            "name": "verifyingContract",
            "type": "address"
          }
        ],
        "OrderComponents": [
          {
            "name": "offerer",
            "type": "address"
          },
          {
            "name": "zone",
            "type": "address"
          },
          {
            "name": "offer",
            "type": "OfferItem[]"
          },
          {
            "name": "consideration",
            "type": "ConsiderationItem[]"
          },
          {
            "name": "orderType",
            "type": "uint8"
          },
          {
            "name": "startTime",
            "type": "uint256"
          },
          {
            "name": "endTime",
            "type": "uint256"
          },
          {
            "name": "zoneHash",
            "type": "bytes32"
          },
          {
            "name": "salt",
            "type": "uint256"
          },
          {
            "name": "conduitKey",
            "type": "bytes32"
          },
          {
            "name": "counter",
            "type": "uint256"
          }
        ],
        "OfferItem": [
          {
            "name": "itemType",
            "type": "uint8"
          },
          {
            "name": "token",
            "type": "address"
          },
          {
            "name": "identifierOrCriteria",
            "type": "uint256"
          },
          {
            "name": "startAmount",
            "type": "uint256"
          },
          {
            "name": "endAmount",
            "type": "uint256"
          }
        ],
        "ConsiderationItem": [
          {
            "name": "itemType",
            "type": "uint8"
          },
          {
            "name": "token",
            "type": "address"
          },
          {
            "name": "identifierOrCriteria",
            "type": "uint256"
          },
          {
            "name": "startAmount",
            "type": "uint256"
          },
          {
            "name": "endAmount",
            "type": "uint256"
          },
          {
            "name": "recipient",
            "type": "address"
          }
        ]
      },
      "primaryType": "OrderComponents",
      "domain": {
        "name": "Seaport",
        "version": "1.1",
        "chainId": "137",
        "verifyingContract": "0x00000000006c3852cbEf3e08E8dF289169EdE581"
      },
      "message": {
        "offerer": "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266",
        "offer": [
          {
            "itemType": "3",
            "token": "0xA604060890923Ff400e8c6f5290461A83AEDACec",
            "identifierOrCriteria": "110194434039389003190498847789203126033799499726478230611233094448886344768909",
            "startAmount": "1",
            "endAmount": "1"
          }
        ],
        "consideration": [
          {
            "itemType": "0",
            "token": "0x0000000000000000000000000000000000000000",
            "identifierOrCriteria": "0",
            "startAmount": "487500000000000000",
            "endAmount": "487500000000000000",
            "recipient": "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"
          },
          {
            "itemType": "0",
            "token": "0x0000000000000000000000000000000000000000",
            "identifierOrCriteria": "0",
            "startAmount": "12500000000000000",
            "endAmount": "12500000000000000",
            "recipient": "0x8De9C5A032463C561423387a9648c5C7BCC5BC90"
          }
        ],
        "startTime": "1658645591",
        "endTime": "1659250386",
        "orderType": "3",
        "zone": "0x004C00500000aD104D7DBd00e3ae0A5C00560C00",
        "zoneHash": "0x0000000000000000000000000000000000000000000000000000000000000000",
        "salt": "16178208897136618",
        "conduitKey": "0x0000007b02230091a7ed01230072f7006a004d60a8d4e71d599b8104250f0000",
        "totalOriginalConsiderationItems": "2",
        "counter": "0"
      }
    }
            );

    let typed_data: TypedData = serde_json::from_value(json).unwrap();

    // `curl -X POST http://localhost:8545 -d '{"jsonrpc": "2.0", "method": "eth_signTypedData_v4", "params": ["0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266", "{\"types\":{\"EIP712Domain\":[{\"name\":\"name\",\"type\":\"string\"},{\"name\":\"version\",\"type\":\"string\"},{\"name\":\"chainId\",\"type\":\"uint256\"},{\"name\":\"verifyingContract\",\"type\":\"address\"}],\"OrderComponents\":[{\"name\":\"offerer\",\"type\":\"address\"},{\"name\":\"zone\",\"type\":\"address\"},{\"name\":\"offer\",\"type\":\"OfferItem[]\"},{\"name\":\"consideration\",\"type\":\"ConsiderationItem[]\"},{\"name\":\"orderType\",\"type\":\"uint8\"},{\"name\":\"startTime\",\"type\":\"uint256\"},{\"name\":\"endTime\",\"type\":\"uint256\"},{\"name\":\"zoneHash\",\"type\":\"bytes32\"},{\"name\":\"salt\",\"type\":\"uint256\"},{\"name\":\"conduitKey\",\"type\":\"bytes32\"},{\"name\":\"counter\",\"type\":\"uint256\"}],\"OfferItem\":[{\"name\":\"itemType\",\"type\":\"uint8\"},{\"name\":\"token\",\"type\":\"address\"},{\"name\":\"identifierOrCriteria\",\"type\":\"uint256\"},{\"name\":\"startAmount\",\"type\":\"uint256\"},{\"name\":\"endAmount\",\"type\":\"uint256\"}],\"ConsiderationItem\":[{\"name\":\"itemType\",\"type\":\"uint8\"},{\"name\":\"token\",\"type\":\"address\"},{\"name\":\"identifierOrCriteria\",\"type\":\"uint256\"},{\"name\":\"startAmount\",\"type\":\"uint256\"},{\"name\":\"endAmount\",\"type\":\"uint256\"},{\"name\":\"recipient\",\"type\":\"address\"}]},\"primaryType\":\"OrderComponents\",\"domain\":{\"name\":\"Seaport\",\"version\":\"1.1\",\"chainId\":\"137\",\"verifyingContract\":\"0x00000000006c3852cbEf3e08E8dF289169EdE581\"},\"message\":{\"offerer\":\"0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266\",\"offer\":[{\"itemType\":\"3\",\"token\":\"0xA604060890923Ff400e8c6f5290461A83AEDACec\",\"identifierOrCriteria\":\"110194434039389003190498847789203126033799499726478230611233094448886344768909\",\"startAmount\":\"1\",\"endAmount\":\"1\"}],\"consideration\":[{\"itemType\":\"0\",\"token\":\"0x0000000000000000000000000000000000000000\",\"identifierOrCriteria\":\"0\",\"startAmount\":\"487500000000000000\",\"endAmount\":\"487500000000000000\",\"recipient\":\"0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266\"},{\"itemType\":\"0\",\"token\":\"0x0000000000000000000000000000000000000000\",\"identifierOrCriteria\":\"0\",\"startAmount\":\"12500000000000000\",\"endAmount\":\"12500000000000000\",\"recipient\":\"0x8De9C5A032463C561423387a9648c5C7BCC5BC90\"}],\"startTime\":\"1658645591\",\"endTime\":\"1659250386\",\"orderType\":\"3\",\"zone\":\"0x004C00500000aD104D7DBd00e3ae0A5C00560C00\",\"zoneHash\":\"0x0000000000000000000000000000000000000000000000000000000000000000\",\"salt\":\"16178208897136618\",\"conduitKey\":\"0x0000007b02230091a7ed01230072f7006a004d60a8d4e71d599b8104250f0000\",\"totalOriginalConsiderationItems\":\"2\",\"counter\":\"0\"}}"], "id": "1"}' -H "Content-Type: application/json"`

    let signature = api
        .sign_typed_data_v4(
            "0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266".parse().unwrap(),
            &typed_data,
        )
        .await
        .unwrap();

    assert_eq!(
    signature,
    "0xed9afe7f377155ee3a42b25b696d79b55d441aeac7790b97a51b54ad0569b9665ea30bf8e8df12d6ee801c4dcb85ecfb8b23a6f7ae166d5af9acac9befb905451c".to_string()
  );
}
