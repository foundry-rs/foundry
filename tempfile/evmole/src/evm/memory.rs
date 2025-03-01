use super::element::Element;
use std::fmt;

#[derive(Clone)]
pub struct LabeledVec<T> {
    pub data: Vec<u8>,
    pub label: Option<T>,
}

#[derive(Clone)]
pub struct Memory<T> {
    pub data: Vec<(u32, LabeledVec<T>)>,
}

impl<T: fmt::Debug> fmt::Debug for Memory<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} elems:", self.data.len())?;
        for (off, val) in &self.data {
            write!(
                f,
                "\n  - {}: {} | {:?}",
                off,
                val.data
                    .iter()
                    .map(|x| format!("{:02x}", x))
                    .collect::<Vec<_>>()
                    .join(""),
                val.label
            )?;
        }
        Ok(())
    }
}

impl<T> Memory<T>
where
    T: fmt::Debug + Clone + Eq,
{
    pub fn new() -> Self {
        Self { data: Vec::new() }
    }
    pub fn store(&mut self, offset: u32, data: Vec<u8>, label: Option<T>) {
        self.data.push((offset, LabeledVec { data, label }));
    }

    pub fn size(&self) -> usize {
        self.data
            .iter()
            .map(|(off, el)| *off as usize + el.data.len())
            .max()
            .unwrap_or(0)
    }

    pub fn get_mut(&mut self, offset: u32) -> Option<&mut LabeledVec<T>> {
        if let Some(el) = self.data.iter_mut().rev().find(|v| v.0 == offset) {
            return Some(&mut el.1);
        }
        None
    }

    pub fn load(&self, offset: u32) -> (Element<T>, Vec<T>) {
        let mut r = Element {
            data: [0; 32],
            label: None,
        };
        let mut used: Vec<T> = Vec::new();

        #[allow(clippy::needless_range_loop)]
        for idx in 0usize..32 {
            let i = idx as u32 + offset;
            for (off, el) in self.data.iter().rev() {
                if i >= *off && i < *off + el.data.len() as u32 {
                    if let Some(label) = &el.label {
                        if used.last().map_or(true, |last| last != label) {
                            used.push(label.clone());
                        }
                    }
                    // early return if it's one full element
                    if idx == 0 && offset == *off && el.data.len() == 32 {
                        r.data.copy_from_slice(&el.data);
                        r.label = el.label.clone();
                        return (r, used);
                    }
                    r.data[idx] = el.data[(i - off) as usize];
                    break;
                }
            }
        }
        (r, used)
    }
}
