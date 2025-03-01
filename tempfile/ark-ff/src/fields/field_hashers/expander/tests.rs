use libtest_mimic::{run, Arguments, Failed, Trial};

use sha2::{Sha256, Sha384, Sha512};
use sha3::{Shake128, Shake256};
use std::{
    fs::{read_dir, File},
    io::BufReader,
};

use super::{Expander, ExpanderXmd, ExpanderXof};

#[derive(Debug, serde_derive::Serialize, serde_derive::Deserialize)]
pub struct ExpanderVector {
    #[serde(rename = "DST")]
    pub dst: String,
    pub k: usize,
    pub hash: String,
    pub name: String,
    #[serde(rename = "tests")]
    pub vectors: Vec<TestExpander>,
}

#[derive(Debug, serde_derive::Serialize, serde_derive:: Deserialize)]
pub struct TestExpander {
    #[serde(rename = "DST_prime")]
    pub dst_prime: String,
    pub len_in_bytes: String,
    pub msg: String,
    pub msg_prime: String,
    pub uniform_bytes: String,
}

#[test]
fn expander() {
    let args = Arguments::from_args();
    let mut tests = Vec::<Trial>::new();

    for filename in read_dir("./src/fields/field_hashers/expander/testdata").unwrap() {
        let ff = filename.unwrap();
        let file = File::open(ff.path()).unwrap();
        let u: ExpanderVector = serde_json::from_reader(BufReader::new(file)).unwrap();

        tests.push(Trial::test(
            ff.file_name().to_str().unwrap().to_string(),
            move || do_test(u),
        ));
    }

    run(&args, tests).exit_if_failed();
}

#[derive(Copy, Clone)]
pub enum ExpID {
    XMD(HashID),
    XOF(XofID),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum HashID {
    SHA256,
    SHA384,
    SHA512,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum XofID {
    SHAKE128,
    SHAKE256,
}

fn do_test(data: ExpanderVector) -> Result<(), Failed> {
    let exp_id = match data.hash.as_str() {
        "SHA256" => ExpID::XMD(HashID::SHA256),
        "SHA384" => ExpID::XMD(HashID::SHA384),
        "SHA512" => ExpID::XMD(HashID::SHA512),
        "SHAKE128" => ExpID::XOF(XofID::SHAKE128),
        "SHAKE256" => ExpID::XOF(XofID::SHAKE256),
        _ => unimplemented!(),
    };
    let exp = get_expander(exp_id, data.dst.as_bytes(), data.k);
    for v in data.vectors.iter() {
        let len = usize::from_str_radix(v.len_in_bytes.trim_start_matches("0x"), 16).unwrap();
        let got = exp.expand(v.msg.as_bytes(), len);
        let want = hex::decode(&v.uniform_bytes).unwrap();
        if got != want {
            return Err(format!(
                "Expander: {}\nVector:   {}\ngot:  {:?}\nwant: {:?}",
                data.hash, v.msg, got, want,
            )
            .into());
        }
    }
    Ok(())
}

fn get_expander(id: ExpID, _dst: &[u8], k: usize) -> Box<dyn Expander> {
    let dst = _dst.to_vec();

    match id {
        ExpID::XMD(h) => match h {
            HashID::SHA256 => Box::new(ExpanderXmd {
                hasher: Sha256::default(),
                block_size: 64,
                dst,
            }),
            HashID::SHA384 => Box::new(ExpanderXmd {
                hasher: Sha384::default(),
                block_size: 128,
                dst,
            }),
            HashID::SHA512 => Box::new(ExpanderXmd {
                hasher: Sha512::default(),
                block_size: 128,
                dst,
            }),
        },
        ExpID::XOF(x) => match x {
            XofID::SHAKE128 => Box::new(ExpanderXof {
                xofer: Shake128::default(),
                k,
                dst,
            }),
            XofID::SHAKE256 => Box::new(ExpanderXof {
                xofer: Shake256::default(),
                k,
                dst,
            }),
        },
    }
}
