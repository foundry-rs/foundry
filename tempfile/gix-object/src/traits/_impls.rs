use crate::{Kind, WriteTo};
use gix_hash::ObjectId;
use std::io::Read;
use std::ops::Deref;
use std::rc::Rc;
use std::sync::Arc;

impl<T> crate::Write for &T
where
    T: crate::Write,
{
    fn write(&self, object: &dyn WriteTo) -> Result<ObjectId, crate::write::Error> {
        (*self).write(object)
    }

    fn write_buf(&self, object: Kind, from: &[u8]) -> Result<ObjectId, crate::write::Error> {
        (*self).write_buf(object, from)
    }

    fn write_stream(&self, kind: Kind, size: u64, from: &mut dyn Read) -> Result<ObjectId, crate::write::Error> {
        (*self).write_stream(kind, size, from)
    }
}

impl<T> crate::Write for Arc<T>
where
    T: crate::Write,
{
    fn write(&self, object: &dyn WriteTo) -> Result<ObjectId, crate::write::Error> {
        self.deref().write(object)
    }

    fn write_buf(&self, object: Kind, from: &[u8]) -> Result<ObjectId, crate::write::Error> {
        self.deref().write_buf(object, from)
    }

    fn write_stream(&self, kind: Kind, size: u64, from: &mut dyn Read) -> Result<ObjectId, crate::write::Error> {
        self.deref().write_stream(kind, size, from)
    }
}

impl<T> crate::Write for Rc<T>
where
    T: crate::Write,
{
    fn write(&self, object: &dyn WriteTo) -> Result<ObjectId, crate::write::Error> {
        self.deref().write(object)
    }

    fn write_buf(&self, object: Kind, from: &[u8]) -> Result<ObjectId, crate::write::Error> {
        self.deref().write_buf(object, from)
    }

    fn write_stream(&self, kind: Kind, size: u64, from: &mut dyn Read) -> Result<ObjectId, crate::write::Error> {
        self.deref().write_stream(kind, size, from)
    }
}

impl<T> WriteTo for &T
where
    T: WriteTo,
{
    fn write_to(&self, out: &mut dyn std::io::Write) -> std::io::Result<()> {
        <T as WriteTo>::write_to(self, out)
    }

    fn kind(&self) -> Kind {
        <T as WriteTo>::kind(self)
    }

    fn size(&self) -> u64 {
        <T as WriteTo>::size(self)
    }
}
