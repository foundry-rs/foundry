#![allow(dead_code)]

use crate::{Blob, Bytes32, Bytes48, Error};
use serde::Deserialize;

#[derive(Deserialize)]
pub struct Input<'a> {
    blob: &'a str,
    z: &'a str,
}

impl Input<'_> {
    pub fn get_blob(&self) -> Result<Blob, Error> {
        Blob::from_hex(self.blob)
    }

    pub fn get_z(&self) -> Result<Bytes32, Error> {
        Bytes32::from_hex(self.z)
    }
}

#[derive(Deserialize)]
pub struct Test<'a> {
    #[serde(borrow)]
    pub input: Input<'a>,
    #[serde(borrow)]
    output: Option<(&'a str, &'a str)>,
}

impl Test<'_> {
    pub fn get_output(&self) -> Option<(Bytes48, Bytes32)> {
        self.output.map(|(proof, y)| {
            (
                Bytes48::from_hex(proof).unwrap(),
                Bytes32::from_hex(y).unwrap(),
            )
        })
    }
}
