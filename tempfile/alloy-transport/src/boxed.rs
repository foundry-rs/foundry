use crate::{Transport, TransportError, TransportFut};
use alloy_json_rpc::{RequestPacket, ResponsePacket};
use std::{any::TypeId, fmt};
use tower::Service;

#[allow(unnameable_types)]
mod private {
    pub trait Sealed {}
    impl<T: super::Transport + Clone> Sealed for T {}
}

/// Trait for converting a transport into a boxed transport.
///
/// This trait is sealed and implemented for all types that implement
/// [`Transport`] + [`Clone`].
pub trait IntoBoxTransport: Transport + Clone + private::Sealed {
    /// Boxes the transport.
    fn into_box_transport(self) -> BoxTransport;
}

impl<T: Transport + Clone> IntoBoxTransport for T {
    fn into_box_transport(self) -> BoxTransport {
        // "specialization" to re-use `BoxTransport`.
        if TypeId::of::<T>() == TypeId::of::<BoxTransport>() {
            // This is not `transmute` because it doesn't allow size mismatch at compile time.
            // `transmute_copy` is a work-around for `transmute_unchecked` not being stable.
            // SAFETY: `self` is `BoxTransport`. This is a no-op.
            let this = std::mem::ManuallyDrop::new(self);
            return unsafe { std::mem::transmute_copy(&this) };
        }
        BoxTransport { inner: Box::new(self) }
    }
}

/// A boxed, Clone-able [`Transport`] trait object.
///
/// This type allows RPC clients to use a type-erased transport. It is
/// [`Clone`] and [`Send`] + [`Sync`], and implements [`Transport`]. This
/// allows for complex behavior abstracting across several different clients
/// with different transport types.
///
/// All higher-level types, such as `RpcClient`, use this type internally
/// rather than a generic [`Transport`] parameter.
pub struct BoxTransport {
    inner: Box<dyn CloneTransport>,
}

impl BoxTransport {
    /// Instantiate a new box transport from a suitable transport.
    #[inline]
    pub fn new<T: IntoBoxTransport>(transport: T) -> Self {
        transport.into_box_transport()
    }

    /// Returns a reference to the inner transport.
    #[inline]
    pub fn as_any(&self) -> &dyn std::any::Any {
        self.inner.as_any()
    }
}

impl fmt::Debug for BoxTransport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BoxTransport").finish_non_exhaustive()
    }
}

impl Clone for BoxTransport {
    fn clone(&self) -> Self {
        Self { inner: self.inner.clone_box() }
    }
}

/// Helper trait for constructing [`BoxTransport`].
trait CloneTransport: Transport + std::any::Any {
    fn clone_box(&self) -> Box<dyn CloneTransport + Send + Sync>;
    fn as_any(&self) -> &dyn std::any::Any;
}

impl<T> CloneTransport for T
where
    T: Transport + Clone + Send + Sync,
{
    #[inline]
    fn clone_box(&self) -> Box<dyn CloneTransport + Send + Sync> {
        Box::new(self.clone())
    }

    #[inline]
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl Service<RequestPacket> for BoxTransport {
    type Response = ResponsePacket;
    type Error = TransportError;
    type Future = TransportFut<'static>;

    #[inline]
    fn poll_ready(
        &mut self,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    #[inline]
    fn call(&mut self, req: RequestPacket) -> Self::Future {
        self.inner.call(req)
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[derive(Clone)]
    struct DummyTransport<T>(T);
    impl<T> Service<RequestPacket> for DummyTransport<T> {
        type Response = ResponsePacket;
        type Error = TransportError;
        type Future = TransportFut<'static>;

        fn poll_ready(
            &mut self,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Result<(), Self::Error>> {
            unimplemented!()
        }

        fn call(&mut self, _req: RequestPacket) -> Self::Future {
            unimplemented!()
        }
    }

    // checks trait + send + sync + 'static
    const fn _compile_check() {
        const fn inner<T>()
        where
            T: Transport + CloneTransport + Send + Sync + Clone + IntoBoxTransport + 'static,
        {
        }
        inner::<BoxTransport>();
    }

    #[test]
    fn no_reboxing() {
        let id = TypeId::of::<DummyTransport<()>>();
        no_reboxing_(DummyTransport(()), id);
        no_reboxing_(BoxTransport::new(DummyTransport(())), id);

        let wrap = String::from("hello");
        let id = TypeId::of::<DummyTransport<String>>();
        no_reboxing_(DummyTransport(wrap.clone()), id);
        no_reboxing_(BoxTransport::new(DummyTransport(wrap)), id);
    }

    fn no_reboxing_<T: IntoBoxTransport>(t: T, id: TypeId) {
        eprintln!("{}", std::any::type_name::<T>());

        let t1 = BoxTransport::new(t);
        let t1p = std::ptr::addr_of!(*t1.inner);
        let t1id = t1.as_any().type_id();

        // This shouldn't wrap `t1` in another box (`BoxTransport<BoxTransport<_>>`).
        let t2 = BoxTransport::new(t1);
        let t2p = std::ptr::addr_of!(*t2.inner);
        let t2id = t2.as_any().type_id();

        assert_eq!(t1id, id);
        assert_eq!(t1id, t2id);
        assert!(std::ptr::eq(t1p, t2p));
    }
}
