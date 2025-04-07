macro_rules! write_chunk {
    ($self:expr, $format_str:literal) => {{
        write_chunk!($self, $format_str,)
    }};
    ($self:expr, $format_str:literal, $($arg:tt)*) => {{
        $self.write_chunk(&format!($format_str, $($arg)*).into())
    }};
    ($self:expr, $loc:expr) => {{
        write_chunk!($self, $loc, "")
    }};
    ($self:expr, $loc:expr, $format_str:literal) => {{
        write_chunk!($self, $loc, $format_str,)
    }};
    ($self:expr, $loc:expr, $format_str:literal, $($arg:tt)*) => {{
        let chunk = $self.chunk_at($loc, None, None, format_args!($format_str, $($arg)*),);
        $self.write_chunk(&chunk)
    }};
    ($self:expr, $loc:expr, $end_loc:expr, $format_str:literal) => {{
        write_chunk!($self, $loc, $end_loc, $format_str,)
    }};
    ($self:expr, $loc:expr, $end_loc:expr, $format_str:literal, $($arg:tt)*) => {{
        let chunk = $self.chunk_at($loc, Some($end_loc), None, format_args!($format_str, $($arg)*),);
        $self.write_chunk(&chunk)
    }};
    ($self:expr, $loc:expr, $end_loc:expr, $needs_space:expr, $format_str:literal, $($arg:tt)*) => {{
        let chunk = $self.chunk_at($loc, Some($end_loc), Some($needs_space), format_args!($format_str, $($arg)*),);
        $self.write_chunk(&chunk)
    }};
}

macro_rules! writeln_chunk {
    ($self:expr) => {{
        writeln_chunk!($self, "")
    }};
    ($self:expr, $format_str:literal) => {{
        writeln_chunk!($self, $format_str,)
    }};
    ($self:expr, $format_str:literal, $($arg:tt)*) => {{
        write_chunk!($self, "{}\n", format_args!($format_str, $($arg)*))
    }};
    ($self:expr, $loc:expr) => {{
        writeln_chunk!($self, $loc, "")
    }};
    ($self:expr, $loc:expr, $format_str:literal) => {{
        writeln_chunk!($self, $loc, $format_str,)
    }};
    ($self:expr, $loc:expr, $format_str:literal, $($arg:tt)*) => {{
        write_chunk!($self, $loc, "{}\n", format_args!($format_str, $($arg)*))
    }};
    ($self:expr, $loc:expr, $end_loc:expr, $format_str:literal) => {{
        writeln_chunk!($self, $loc, $end_loc, $format_str,)
    }};
    ($self:expr, $loc:expr, $end_loc:expr, $format_str:literal, $($arg:tt)*) => {{
        write_chunk!($self, $loc, $end_loc, "{}\n", format_args!($format_str, $($arg)*))
    }};
}

macro_rules! write_chunk_spaced {
    ($self:expr, $loc:expr, $needs_space:expr, $format_str:literal) => {{
        write_chunk_spaced!($self, $loc, $needs_space, $format_str,)
    }};
    ($self:expr, $loc:expr, $needs_space:expr, $format_str:literal, $($arg:tt)*) => {{
        let chunk = $self.chunk_at($loc, None, $needs_space, format_args!($format_str, $($arg)*),);
        $self.write_chunk(&chunk)
    }};
}

macro_rules! buf_fn {
    ($vis:vis fn $name:ident(&self $(,)? $($arg_name:ident : $arg_ty:ty),*) $(-> $ret:ty)?) => {
        $vis fn $name(&self, $($arg_name : $arg_ty),*) $(-> $ret)? {
            if self.temp_bufs.is_empty() {
                self.buf.$name($($arg_name),*)
            } else {
                self.temp_bufs.last().unwrap().$name($($arg_name),*)
            }
        }
    };
    ($vis:vis fn $name:ident(&mut self $(,)? $($arg_name:ident : $arg_ty:ty),*) $(-> $ret:ty)?) => {
        $vis fn $name(&mut self, $($arg_name : $arg_ty),*) $(-> $ret)? {
            if self.temp_bufs.is_empty() {
                self.buf.$name($($arg_name),*)
            } else {
                self.temp_bufs.last_mut().unwrap().$name($($arg_name),*)
            }
        }
    };
}

macro_rules! return_source_if_disabled {
    ($self:expr, $loc:expr) => {{
        let loc = $loc;
        if $self.inline_config.is_disabled(loc) {
            trace!("Returning because disabled: {loc:?}");
            return $self.visit_source(loc)
        }
    }};
    ($self:expr, $loc:expr, $suffix:literal) => {{
        let mut loc = $loc;
        let has_suffix = $self.extend_loc_until(&mut loc, $suffix);
        if $self.inline_config.is_disabled(loc) {
            $self.visit_source(loc)?;
            trace!("Returning because disabled: {loc:?}");
            if !has_suffix {
                write!($self.buf(), "{}", $suffix)?;
            }
            return Ok(())
        }
    }};
}

macro_rules! visit_source_if_disabled_else {
    ($self:expr, $loc:expr, $block:block) => {{
        let loc = $loc;
        if $self.inline_config.is_disabled(loc) {
            $self.visit_source(loc)?;
        } else $block
    }};
}

pub(crate) use buf_fn;
pub(crate) use return_source_if_disabled;
pub(crate) use visit_source_if_disabled_else;
pub(crate) use write_chunk;
pub(crate) use write_chunk_spaced;
pub(crate) use writeln_chunk;
