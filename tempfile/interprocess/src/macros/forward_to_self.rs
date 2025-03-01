/// Forwards trait methods to inherent ones with the same name and signature.
macro_rules! forward_to_self {
	(
		fn $mnm:ident $({$($fgen:tt)*})?
		(&self $(, $param:ident : $pty:ty)* $(,)?) $(-> $ret:ty)?
	) => {
		#[inline(always)]
		fn $mnm $(<$($fgen)*>)? (&self, $($param: $pty),*) $(-> $ret)? {
			self.$mnm($($param),*)
		}
	};
	(
		fn $mnm:ident $({$($fgen:tt)*})?
		(&mut self $(, $param:ident : $pty:ty),* $(,)?) $(-> $ret:ty)?
	) => {
		#[inline(always)]
		fn $mnm $(<$($fgen)*>)? (&mut self, $($param: $pty),*) $(-> $ret)? {
			self.$mnm($($param),*)
		}
	};
	(
		fn $mnm:ident $({$($fgen:tt)*})?
		(self $(, $param:ident : $pty:ty),* $(,)?) $(-> $ret:ty)?
	) => {
		#[inline(always)]
		fn $mnm $(<$($fgen)*>)? (self, $($param: $pty),*) $(-> $ret)? {
			self.$mnm($($param),*)
		}
	};
	(fn $mnm:ident $({$($fgen:tt)*})? ($($param:ident : $pty:ty),* $(,)?) $(-> $ret:ty)?) => {
		#[inline(always)]
		fn $mnm $(<$($fgen)*>)? ($($param: $pty),*) $(-> $ret)? {
			Self::$mnm($($param),*)
		}
	};
	($(fn $mnm:ident $({$($fgen:tt)*})? ($($args:tt)*) $(-> $ret:ty)?);+ $(;)?) => {$(
		forward_to_self!(fn $mnm $({$($fgen)*})? ($($args)*) $(-> $ret)?);
	)+};
}
