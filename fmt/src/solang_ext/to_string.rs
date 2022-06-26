use solang_parser::pt::Unit;

pub struct UnitStr<'a>(pub &'a mut Unit);
impl<'a> AsRef<str> for UnitStr<'a> {
    fn as_ref(&self) -> &'a str {
        match self.0 {
            Unit::Seconds(_) => "seconds",
            Unit::Minutes(_) => "minutes",
            Unit::Hours(_) => "hours",
            Unit::Days(_) => "days",
            Unit::Weeks(_) => "weeks",
            Unit::Wei(_) => "wei",
            Unit::Gwei(_) => "gwei",
            Unit::Ether(_) => "ether",
        }
    }
}
