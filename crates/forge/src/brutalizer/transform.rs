use solar::ast::Span;

pub(super) enum Transform {
    Insert { offset: usize, replacement: String },
    Replace { span: Span, replacement: String },
}

impl Transform {
    fn start(&self) -> usize {
        match self {
            Self::Insert { offset, .. } => *offset,
            Self::Replace { span, .. } => span.lo().0 as usize,
        }
    }

    fn end(&self) -> usize {
        match self {
            Self::Insert { offset, .. } => *offset,
            Self::Replace { span, .. } => span.hi().0 as usize,
        }
    }
}

pub(super) fn span_text(source: &str, span: Span) -> Option<&str> {
    let lo = span.lo().0 as usize;
    let hi = span.hi().0 as usize;
    source.get(lo..hi)
}

pub(super) fn apply_transforms(source: &str, mut transforms: Vec<Transform>) -> String {
    transforms.sort_by(|a, b| b.start().cmp(&a.start()).then_with(|| b.end().cmp(&a.end())));

    let mut result = source.to_string();
    for transform in transforms {
        match transform {
            Transform::Insert { offset, replacement } => result.insert_str(offset, &replacement),
            Transform::Replace { span, replacement } => {
                let lo = span.lo().0 as usize;
                let hi = span.hi().0 as usize;
                result.replace_range(lo..hi, &replacement);
            }
        }
    }
    result
}
