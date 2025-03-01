use bon::bon;

struct InvalidAttrsForBonMacro;

#[bon(attrs)]
impl InvalidAttrsForBonMacro {
    #[builder]
    fn sut() {}
}

struct BuilderAttrOnReceiver;

#[bon]
impl BuilderAttrOnReceiver {
    #[builder]
    fn sut(#[builder] &self) {}
}

struct NoBuilderMethods;

#[bon]
impl NoBuilderMethods {
    fn not_builder1() {}
    fn not_builder2(&self) {}

    const NOT_BUILDER: () = ();
}


fn main() {}
