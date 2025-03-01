/// Prevents `Counter` from being implemented externally.
///
/// Items exist on this trait rather than `Counter` so that they are impossible
/// to access externally.
pub trait Sealed {}
