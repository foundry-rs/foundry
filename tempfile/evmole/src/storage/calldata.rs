use alloy_dyn_abi::DynSolType;
use alloy_dyn_abi::DynSolType::*;
use std::{collections::BTreeMap, marker::PhantomData};

use crate::evm::{
    calldata::{CallData, CallDataLabel},
    element::Element,
    U256, VAL_131072,
};
use std::error;

pub struct CallDataImpl<T> {
    pub selector: [u8; 4],
    arg_types: BTreeMap<usize, DynSolType>,
    arg_vals: BTreeMap<usize, usize>,

    phantom: std::marker::PhantomData<T>,
}

impl<T> CallDataImpl<T> {
    pub fn new(selector: [u8; 4], arguments: &[DynSolType]) -> Self {
        let (_, types, vals) = encode(arguments);
        Self {
            selector,
            arg_types: BTreeMap::from_iter(types),
            arg_vals: BTreeMap::from_iter(vals),
            phantom: PhantomData,
        }
    }
}

impl<T: CallDataLabel> CallData<T> for CallDataImpl<T> {
    fn load32(&self, offset: U256) -> Element<T> {
        let mut data = [0; 32];
        let mut label = None;

        if let Ok(off) = usize::try_from(offset) {
            if off < 4 {
                data[..4 - off].copy_from_slice(&self.selector[off..]);
            } else {
                let xoff = off - 4;
                if let Some(val) = self.arg_vals.get(&xoff) {
                    data = U256::from(*val).to_be_bytes();
                }
                if let Some(tp) = self.arg_types.get(&xoff) {
                    // label = Some(Label::Typed(tp.clone()));
                    label = Some(T::label(xoff, tp));
                }
            }
        }
        Element { data, label }
    }

    fn load(
        &self,
        offset: U256,
        size: U256,
    ) -> Result<(Vec<u8>, Option<T>), Box<dyn error::Error>> {
        let mut data = vec![0; u8::try_from(size)? as usize]; // max len limited to max_u8
        let mut label = None;

        if let Ok(off) = usize::try_from(offset) {
            if off < 4 {
                let nlen = std::cmp::min(data.len(), 4 - off);
                data[..nlen].copy_from_slice(&self.selector[off..off + nlen]);
            } else {
                if let Some(val) = self.arg_vals.get(&off) {
                    //TODO: look to the left to find proper element
                    data = U256::from(*val).to_be_bytes_vec();
                }
                if let Some(tp) = self.arg_types.get(&off) {
                    label = Some(T::label(off - 4, tp));
                }
            }
        }

        Ok((data, label))
    }

    fn selector(&self) -> [u8; 4] {
        self.selector
    }

    fn len(&self) -> U256 {
        VAL_131072
    }
}

fn is_dynamic(ty: &DynSolType) -> bool {
    match ty {
        Bool | Int(_) | Uint(_) | Address | FixedBytes(_) => false,
        FixedArray(val, _) => is_dynamic(val),
        Bytes | String | Array(_) => true,
        Tuple(val) => val.iter().any(is_dynamic),
        _ => panic!("Unexpected type {:?}", ty),
    }
}

type ArgTypes = Vec<(usize, DynSolType)>;
type ArgNonZero = Vec<(usize, usize)>;

fn encode(elements: &[DynSolType]) -> (usize, ArgTypes, ArgNonZero) {
    // (offset, type)
    let mut ret_types: Vec<(usize, DynSolType)> = Vec::with_capacity(elements.len());

    // (offset, value)
    let mut ret_nonzero: Vec<(usize, usize)> = Vec::with_capacity(elements.len());

    let mut off = 0;

    // (offset, type)
    let mut dynamic: Vec<(usize, &DynSolType)> = Vec::with_capacity(elements.len() / 2);

    for ty in elements.iter() {
        if is_dynamic(ty) {
            dynamic.push((off, ty));
            off += 32;
        } else {
            match ty {
                FixedArray(val, sz) => {
                    for _ in 0..*sz {
                        // sz or cnt?
                        ret_types.push((off, *val.clone()));
                        off += 32;
                    }
                }
                Tuple(val) => {
                    for v in val {
                        ret_types.push((off, v.clone()));
                        off += 32;
                    }
                }

                _ => {
                    ret_types.push((off, ty.clone()));
                    off += 32;
                }
            }
        }
    }

    for (el_off, ty) in dynamic.into_iter() {
        ret_nonzero.push((el_off, off));

        match ty {
            Bytes | String => {
                // string '0x41' with len = 1
                ret_nonzero.push((off, 32));
                ret_nonzero.push((off + 32, 0x41)); // TODO: padd right, not left
                ret_types.push((off + 32, ty.clone()));
                off += 64;
            }
            Array(val) => {
                // len = 1
                ret_nonzero.push((off, 1));
                off += 32;

                let (dyn_off, dyn_ret_types, dyn_ret_nonzero) = encode(&[*val.clone()]);

                ret_types.extend(dyn_ret_types.into_iter().map(|(o, v)| (o + off, v)));
                ret_nonzero.extend(dyn_ret_nonzero.into_iter().map(|(o, v)| (o + off, v)));
                off += dyn_off;
            }
            Tuple(val) => {
                let (dyn_off, dyn_ret_types, dyn_ret_nonzero) = encode(val);

                ret_types.extend(dyn_ret_types.into_iter().map(|(o, v)| (o + off, v)));
                ret_nonzero.extend(dyn_ret_nonzero.into_iter().map(|(o, v)| (o + off, v)));
                off += dyn_off;
            }

            FixedArray(val, sz) => {
                let (dyn_off, dyn_ret_types, dyn_ret_nonzero) = encode(&[*val.clone()]);

                let data_start = 32 * sz;
                for i in 0..*sz {
                    ret_nonzero.push((off, data_start + i * (dyn_off - 32)));
                    off += 32;
                }

                for _ in 0..*sz {
                    ret_types.extend(dyn_ret_types.iter().map(|(o, v)| (o + off - 32, v.clone())));
                    ret_nonzero.extend(
                        dyn_ret_nonzero
                            .iter()
                            .skip(1)
                            .map(|(o, v)| (o + off - 32, *v)),
                    );
                    off += dyn_off - 32;
                }
            }
            _ => panic!("Unexpected type {:?}", ty),
        }
    }

    (off, ret_types, ret_nonzero)
}

// #[cfg(test)]
// mod test {
//     use super::encode;
//     use std::collections::BTreeMap;
//
//     #[test]
//     fn test_encode() {
//         let x = vec![
//             "string[3]".parse().unwrap(),
//             // "(string)[3]".parse().unwrap(),
//             // "(uint8, string)[3]".parse().unwrap(),
//         ];
//         let (end_off, a, b) = encode(&x);
//         println!("{}", end_off);
//         println!("{:?}", a);
//         println!("{:?}", b);
//
//         let ma = BTreeMap::from_iter(a);
//         let mb = BTreeMap::from_iter(b);
//
//         for off in (0..end_off).step_by(32) {
//             println!("{:064x} - {:?}",
//                 mb.get(&off).unwrap_or(&0),
//                 ma.get(&off),
//             );
//         }
//     }
// }
