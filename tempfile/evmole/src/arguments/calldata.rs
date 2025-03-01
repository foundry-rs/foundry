use super::Label;
use crate::evm::{calldata::CallData, element::Element, U256, VAL_4, VAL_131072};
use std::error;

pub(super) struct CallDataImpl {
    pub selector: [u8; 4],
}

impl CallData<Label> for CallDataImpl {
    fn load32(&self, offset: U256) -> Element<Label> {
        let mut data = [0; 32];
        if offset < VAL_4 {
            let off = usize::try_from(offset).expect("len checked");
            data[..4 - off].copy_from_slice(&self.selector[off..]);
        }
        Element {
            data,
            label: Some(Label::CallData),
        }
    }

    fn load(
        &self,
        offset: U256,
        size: U256,
    ) -> Result<(Vec<u8>, Option<Label>), Box<dyn error::Error>> {
        let sz = u16::try_from(size)?;
        if sz > 512 {
            return Err("unsupported size".into());
        }
        let mut data = vec![0; sz as usize];
        if offset < VAL_4 {
            let off = usize::try_from(offset).expect("len checked");
            let nlen = std::cmp::min(data.len(), 4 - off);
            data[..nlen].copy_from_slice(&self.selector[off..off + nlen]);
        }
        Ok((data, Some(Label::CallData)))
    }

    fn selector(&self) -> [u8; 4] {
        self.selector
    }

    fn len(&self) -> U256 {
        VAL_131072
    }
}
