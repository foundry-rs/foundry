use solang_parser::pt::Unit;

pub trait AsStr {
    fn as_str(&self) -> &str;
}

impl AsStr for Unit {
    fn as_str(&self) -> &str {
        match self {
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
