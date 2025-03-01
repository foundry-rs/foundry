// This file is part of ICU4X. For terms of use, please see the file
// called LICENSE at the top level of the ICU4X source tree
// (online at: https://github.com/unicode-org/icu4x/blob/main/LICENSE ).

use crate::buf::BufferMarker;
use crate::error::{DataError, DataErrorKind};
use crate::marker::DataMarker;
use crate::request::DataLocale;
use alloc::boxed::Box;
use core::convert::TryFrom;
use core::fmt::Debug;
use core::marker::PhantomData;
use core::ops::Deref;
use yoke::cartable_ptr::CartableOptionPointer;
use yoke::trait_hack::YokeTraitHack;
use yoke::*;

#[cfg(not(feature = "sync"))]
use alloc::rc::Rc as SelectedRc;
#[cfg(feature = "sync")]
use alloc::sync::Arc as SelectedRc;

/// A response object containing metadata about the returned data.
#[derive(Debug, Clone, PartialEq, Default)]
#[non_exhaustive]
pub struct DataResponseMetadata {
    /// The resolved locale of the returned data, if locale fallbacking was performed.
    pub locale: Option<DataLocale>,
    /// The format of the buffer for buffer-backed data, if known (for example, JSON).
    pub buffer_format: Option<crate::buf::BufferFormat>,
}

/// A container for data payloads returned from a data provider.
///
/// [`DataPayload`] is built on top of the [`yoke`] framework, which allows for cheap, zero-copy
/// operations on data via the use of self-references.
///
/// The type of the data stored in [`DataPayload`] is determined by the [`DataMarker`] type parameter.
///
/// ## Accessing the data
///
/// To get a reference to the data inside [`DataPayload`], use [`DataPayload::get()`]. If you need
/// to store the data for later use, you need to store the [`DataPayload`] itself, since `get` only
/// returns a reference with an ephemeral lifetime.
///
/// ## Mutating the data
///
/// To modify the data stored in a [`DataPayload`], use [`DataPayload::with_mut()`].
///
/// ## Transforming the data to a different type
///
/// To transform a [`DataPayload`] to a different type backed by the same data store (cart), use
/// [`DataPayload::map_project()`] or one of its sister methods.
///
/// # Cargo feature: `sync`
///
/// By default, the payload uses non-concurrent reference counting internally, and hence is neither
/// [`Sync`] nor [`Send`]; if these traits are required, the `sync` Cargo feature can be enabled.
///
/// # Examples
///
/// Basic usage, using the `HelloWorldV1Marker` marker:
///
/// ```
/// use icu_provider::hello_world::*;
/// use icu_provider::prelude::*;
/// use std::borrow::Cow;
///
/// let payload = DataPayload::<HelloWorldV1Marker>::from_owned(HelloWorldV1 {
///     message: Cow::Borrowed("Demo"),
/// });
///
/// assert_eq!("Demo", payload.get().message);
/// ```
pub struct DataPayload<M: DataMarker>(pub(crate) DataPayloadInner<M>);

/// A container for data payloads with storage for something else.
///
/// The type parameter `O` is stored as part of the interior enum, leading to
/// better stack size optimization. `O` can be as large as the [`DataPayload`]
/// minus two words without impacting stack size.
///
/// # Examples
///
/// Create and use DataPayloadOr:
///
/// ```
/// use icu_provider::hello_world::*;
/// use icu_provider::prelude::*;
/// use icu_provider::DataPayloadOr;
///
/// let payload: DataPayload<HelloWorldV1Marker> = HelloWorldProvider
///     .load(DataRequest {
///         locale: &"de".parse().unwrap(),
///         metadata: Default::default(),
///     })
///     .expect("Loading should succeed")
///     .take_payload()
///     .expect("Data should be present");
///
/// let payload_some =
///     DataPayloadOr::<HelloWorldV1Marker, ()>::from_payload(payload);
/// let payload_none = DataPayloadOr::<HelloWorldV1Marker, ()>::from_other(());
///
/// assert_eq!(
///     payload_some.get(),
///     Ok(&HelloWorldV1 {
///         message: "Hallo Welt".into()
///     })
/// );
/// assert_eq!(payload_none.get(), Err(&()));
/// ```
///
/// Stack size comparison:
///
/// ```
/// use core::mem::size_of;
/// use icu_provider::prelude::*;
/// use icu_provider::DataPayloadOr;
///
/// const W: usize = size_of::<usize>();
///
/// // SampleStruct is 3 words:
/// # #[icu_provider::data_struct(SampleStructMarker)]
/// # pub struct SampleStruct<'data>(usize, usize, &'data ());
/// assert_eq!(W * 3, size_of::<SampleStruct>());
///
/// // DataPayload adds a word for a total of 4 words:
/// assert_eq!(W * 4, size_of::<DataPayload<SampleStructMarker>>());
///
/// // Option<DataPayload> balloons to 5 words:
/// assert_eq!(W * 5, size_of::<Option<DataPayload<SampleStructMarker>>>());
///
/// // But, using DataPayloadOr is the same size as DataPayload:
/// assert_eq!(W * 4, size_of::<DataPayloadOr<SampleStructMarker, ()>>());
///
/// // The largest optimized Other type is two words smaller than the DataPayload:
/// assert_eq!(W * 4, size_of::<DataPayloadOr<SampleStructMarker, [usize; 1]>>());
/// assert_eq!(W * 4, size_of::<DataPayloadOr<SampleStructMarker, [usize; 2]>>());
/// assert_eq!(W * 5, size_of::<DataPayloadOr<SampleStructMarker, [usize; 3]>>());
/// ```
#[doc(hidden)] // TODO(#4467): establish this as an internal API
pub struct DataPayloadOr<M: DataMarker, O>(pub(crate) DataPayloadOrInner<M, O>);

pub(crate) enum DataPayloadInner<M: DataMarker> {
    Yoke(Yoke<M::Yokeable, CartableOptionPointer<CartInner>>),
    StaticRef(&'static M::Yokeable),
}

pub(crate) enum DataPayloadOrInner<M: DataMarker, O> {
    Yoke(Yoke<M::Yokeable, CartableOptionPointer<CartInner>>),
    Inner(DataPayloadOrInnerInner<M, O>),
}

pub(crate) enum DataPayloadOrInnerInner<M: DataMarker, O> {
    StaticRef(&'static M::Yokeable),
    Other(O),
}

/// The type of the "cart" that is used by [`DataPayload`].
///
/// This type is public but the inner cart type is private. To create a
/// [`Yoke`] with this cart, use [`Cart::try_make_yoke`]. Then, convert
/// it to a [`DataPayload`] with [`DataPayload::from_yoked_buffer`].
#[derive(Clone, Debug)]
#[allow(clippy::redundant_allocation)] // false positive, it's cheaper to wrap an existing Box in an Rc than to reallocate a huge Rc
pub struct Cart(CartInner);

/// The actual cart type (private typedef).
pub(crate) type CartInner = SelectedRc<Box<[u8]>>;

impl Deref for Cart {
    type Target = Box<[u8]>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
// Safe because both Rc and Arc are StableDeref, and our impl delegates.
unsafe impl stable_deref_trait::StableDeref for Cart {}
// Safe because both Rc and Arc are CloneableCart, and our impl delegates.
unsafe impl yoke::CloneableCart for Cart {}

impl Cart {
    /// Creates a `Yoke<Y, Option<Cart>>` from owned bytes by applying `f`.
    pub fn try_make_yoke<Y, F, E>(cart: Box<[u8]>, f: F) -> Result<Yoke<Y, Option<Self>>, E>
    where
        for<'a> Y: Yokeable<'a>,
        F: FnOnce(&[u8]) -> Result<<Y as Yokeable>::Output, E>,
    {
        Yoke::try_attach_to_cart(SelectedRc::new(cart), |b| f(b))
            // Safe because the cart is only wrapped
            .map(|yoke| unsafe { yoke.replace_cart(Cart) })
            .map(Yoke::wrap_cart_in_option)
    }

    /// Helper function to convert `Yoke<Y, Option<Cart>>` to `Yoke<Y, Option<CartInner>>`.
    #[inline]
    pub(crate) fn unwrap_cart<Y>(yoke: Yoke<Y, Option<Cart>>) -> Yoke<Y, Option<CartInner>>
    where
        for<'a> Y: Yokeable<'a>,
    {
        // Safety: `Cart` has one field and we are removing it from the newtype,
        // and we are preserving it in the new cart, unwrapping it from the newtype.
        unsafe { yoke.replace_cart(|option_cart| option_cart.map(|cart| cart.0)) }
    }
}

impl<M> Debug for DataPayload<M>
where
    M: DataMarker,
    for<'a> &'a <M::Yokeable as Yokeable<'a>>::Output: Debug,
{
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.get().fmt(f)
    }
}

impl<M, O> Debug for DataPayloadOr<M, O>
where
    M: DataMarker,
    for<'a> &'a <M::Yokeable as Yokeable<'a>>::Output: Debug,
    O: Debug,
{
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.get()
            .map(|v| Debug::fmt(&v, f))
            .unwrap_or_else(|v| Debug::fmt(v, f))
    }
}

/// Cloning a DataPayload is generally a cheap operation.
/// See notes in the `Clone` impl for [`Yoke`].
///
/// # Examples
///
/// ```no_run
/// use icu_provider::hello_world::*;
/// use icu_provider::prelude::*;
///
/// let resp1: DataPayload<HelloWorldV1Marker> = todo!();
/// let resp2 = resp1.clone();
/// ```
impl<M> Clone for DataPayload<M>
where
    M: DataMarker,
    for<'a> YokeTraitHack<<M::Yokeable as Yokeable<'a>>::Output>: Clone,
{
    fn clone(&self) -> Self {
        Self(match &self.0 {
            DataPayloadInner::Yoke(yoke) => DataPayloadInner::Yoke(yoke.clone()),
            DataPayloadInner::StaticRef(r) => DataPayloadInner::StaticRef(*r),
        })
    }
}

impl<M, O> Clone for DataPayloadOr<M, O>
where
    M: DataMarker,
    for<'a> YokeTraitHack<<M::Yokeable as Yokeable<'a>>::Output>: Clone,
    O: Clone,
{
    fn clone(&self) -> Self {
        Self(match &self.0 {
            DataPayloadOrInner::Yoke(yoke) => DataPayloadOrInner::Yoke(yoke.clone()),
            DataPayloadOrInner::Inner(DataPayloadOrInnerInner::StaticRef(r)) => {
                DataPayloadOrInner::Inner(DataPayloadOrInnerInner::StaticRef(*r))
            }
            DataPayloadOrInner::Inner(DataPayloadOrInnerInner::Other(o)) => {
                DataPayloadOrInner::Inner(DataPayloadOrInnerInner::Other(o.clone()))
            }
        })
    }
}

impl<M> PartialEq for DataPayload<M>
where
    M: DataMarker,
    for<'a> YokeTraitHack<<M::Yokeable as Yokeable<'a>>::Output>: PartialEq,
{
    fn eq(&self, other: &Self) -> bool {
        YokeTraitHack(self.get()).into_ref() == YokeTraitHack(other.get()).into_ref()
    }
}

impl<M, O> PartialEq for DataPayloadOr<M, O>
where
    M: DataMarker,
    for<'a> YokeTraitHack<<M::Yokeable as Yokeable<'a>>::Output>: PartialEq,
    O: Eq,
{
    fn eq(&self, other: &Self) -> bool {
        match (self.get(), other.get()) {
            (Ok(x), Ok(y)) => YokeTraitHack(x).into_ref() == YokeTraitHack(y).into_ref(),
            (Err(x), Err(y)) => x == y,
            _ => false,
        }
    }
}

impl<M> Eq for DataPayload<M>
where
    M: DataMarker,
    for<'a> YokeTraitHack<<M::Yokeable as Yokeable<'a>>::Output>: Eq,
{
}

impl<M, O> Eq for DataPayloadOr<M, O>
where
    M: DataMarker,
    for<'a> YokeTraitHack<<M::Yokeable as Yokeable<'a>>::Output>: Eq,
    O: Eq,
{
}

#[test]
fn test_clone_eq() {
    use crate::hello_world::*;
    let p1 = DataPayload::<HelloWorldV1Marker>::from_static_str("Demo");
    #[allow(clippy::redundant_clone)]
    let p2 = p1.clone();
    assert_eq!(p1, p2);

    let p1 = DataPayloadOr::<HelloWorldV1Marker, usize>::from_payload(p1);
    #[allow(clippy::redundant_clone)]
    let p2 = p1.clone();
    assert_eq!(p1, p2);

    let p3 = DataPayloadOr::<HelloWorldV1Marker, usize>::from_other(555);
    #[allow(clippy::redundant_clone)]
    let p4 = p3.clone();
    assert_eq!(p3, p4);

    let p5 = DataPayloadOr::<HelloWorldV1Marker, usize>::from_other(666);
    assert_ne!(p3, p5);
    assert_ne!(p4, p5);

    assert_ne!(p1, p3);
    assert_ne!(p1, p4);
    assert_ne!(p1, p5);
    assert_ne!(p2, p3);
    assert_ne!(p2, p4);
    assert_ne!(p2, p5);
}

impl<M> DataPayload<M>
where
    M: DataMarker,
{
    /// Convert a fully owned (`'static`) data struct into a DataPayload.
    ///
    /// This constructor creates `'static` payloads.
    ///
    /// # Examples
    ///
    /// ```
    /// use icu_provider::hello_world::*;
    /// use icu_provider::prelude::*;
    /// use std::borrow::Cow;
    ///
    /// let local_struct = HelloWorldV1 {
    ///     message: Cow::Owned("example".to_owned()),
    /// };
    ///
    /// let payload =
    ///     DataPayload::<HelloWorldV1Marker>::from_owned(local_struct.clone());
    ///
    /// assert_eq!(payload.get(), &local_struct);
    /// ```
    #[inline]
    pub fn from_owned(data: M::Yokeable) -> Self {
        Self(DataPayloadInner::Yoke(
            Yoke::new_owned(data).convert_cart_into_option_pointer(),
        ))
    }

    #[doc(hidden)]
    #[inline]
    pub const fn from_static_ref(data: &'static M::Yokeable) -> Self {
        Self(DataPayloadInner::StaticRef(data))
    }

    /// Convert a DataPayload that was created via [`DataPayload::from_owned()`] back into the
    /// concrete type used to construct it.
    pub fn try_unwrap_owned(self) -> Result<M::Yokeable, DataError> {
        match self.0 {
            DataPayloadInner::Yoke(yoke) => yoke.try_into_yokeable().ok(),
            DataPayloadInner::StaticRef(_) => None,
        }
        .ok_or(DataErrorKind::InvalidState.with_str_context("try_unwrap_owned"))
    }

    /// Mutate the data contained in this DataPayload.
    ///
    /// For safety, all mutation operations must take place within a helper function that cannot
    /// borrow data from the surrounding context.
    ///
    /// # Examples
    ///
    /// Basic usage:
    ///
    /// ```
    /// use icu_provider::hello_world::HelloWorldV1Marker;
    /// use icu_provider::prelude::*;
    ///
    /// let mut payload =
    ///     DataPayload::<HelloWorldV1Marker>::from_static_str("Hello");
    ///
    /// payload.with_mut(|s| s.message.to_mut().push_str(" World"));
    ///
    /// assert_eq!("Hello World", payload.get().message);
    /// ```
    ///
    /// To transfer data from the context into the data struct, use the `move` keyword:
    ///
    /// ```
    /// use icu_provider::hello_world::HelloWorldV1Marker;
    /// use icu_provider::prelude::*;
    ///
    /// let mut payload =
    ///     DataPayload::<HelloWorldV1Marker>::from_static_str("Hello");
    ///
    /// let suffix = " World";
    /// payload.with_mut(move |s| s.message.to_mut().push_str(suffix));
    ///
    /// assert_eq!("Hello World", payload.get().message);
    /// ```
    pub fn with_mut<'a, F>(&'a mut self, f: F)
    where
        F: 'static + for<'b> FnOnce(&'b mut <M::Yokeable as Yokeable<'a>>::Output),
        M::Yokeable: zerofrom::ZeroFrom<'static, M::Yokeable>,
    {
        if let DataPayloadInner::StaticRef(r) = self.0 {
            self.0 = DataPayloadInner::Yoke(
                Yoke::new_owned(zerofrom::ZeroFrom::zero_from(r))
                    .convert_cart_into_option_pointer(),
            );
        }
        match &mut self.0 {
            DataPayloadInner::Yoke(yoke) => yoke.with_mut(f),
            _ => unreachable!(),
        }
    }

    /// Borrows the underlying data.
    ///
    /// This function should be used like `Deref` would normally be used. For more information on
    /// why DataPayload cannot implement `Deref`, see the `yoke` crate.
    ///
    /// # Examples
    ///
    /// ```
    /// use icu_provider::hello_world::HelloWorldV1Marker;
    /// use icu_provider::prelude::*;
    ///
    /// let payload = DataPayload::<HelloWorldV1Marker>::from_static_str("Demo");
    ///
    /// assert_eq!("Demo", payload.get().message);
    /// ```
    #[inline]
    #[allow(clippy::needless_lifetimes)]
    pub fn get<'a>(&'a self) -> &'a <M::Yokeable as Yokeable<'a>>::Output {
        match &self.0 {
            DataPayloadInner::Yoke(yoke) => yoke.get(),
            DataPayloadInner::StaticRef(r) => Yokeable::transform(*r),
        }
    }

    /// Maps `DataPayload<M>` to `DataPayload<M2>` by projecting it with [`Yoke::map_project`].
    ///
    /// This is accomplished by a function that takes `M`'s data type and returns `M2`'s data
    /// type. The function takes a second argument which should be ignored. For more details,
    /// see [`Yoke::map_project()`].
    ///
    /// The standard [`DataPayload::map_project()`] function moves `self` and cannot capture any
    /// data from its context. Use one of the sister methods if you need these capabilities:
    ///
    /// - [`DataPayload::map_project_cloned()`] if you don't have ownership of `self`
    /// - [`DataPayload::try_map_project()`] to bubble up an error
    /// - [`DataPayload::try_map_project_cloned()`] to do both of the above
    ///
    /// # Examples
    ///
    /// Map from `HelloWorldV1` to a `Cow<str>` containing just the message:
    ///
    /// ```
    /// use icu_provider::hello_world::*;
    /// use icu_provider::prelude::*;
    /// use std::borrow::Cow;
    ///
    /// // A custom marker type is required when using `map_project`. The Yokeable should be the
    /// // target type, and the Cart should correspond to the type being transformed.
    ///
    /// struct HelloWorldV1MessageMarker;
    /// impl DataMarker for HelloWorldV1MessageMarker {
    ///     type Yokeable = Cow<'static, str>;
    /// }
    ///
    /// let p1: DataPayload<HelloWorldV1Marker> = DataPayload::from_owned(HelloWorldV1 {
    ///     message: Cow::Borrowed("Hello World"),
    /// });
    ///
    /// assert_eq!("Hello World", p1.get().message);
    ///
    /// let p2: DataPayload<HelloWorldV1MessageMarker> = p1.map_project(|obj, _| obj.message);
    ///
    /// // Note: at this point, p1 has been moved.
    /// assert_eq!("Hello World", p2.get());
    /// ```
    #[allow(clippy::type_complexity)]
    pub fn map_project<M2, F>(self, f: F) -> DataPayload<M2>
    where
        M2: DataMarker,
        F: for<'a> FnOnce(
            <M::Yokeable as Yokeable<'a>>::Output,
            PhantomData<&'a ()>,
        ) -> <M2::Yokeable as Yokeable<'a>>::Output,
        M::Yokeable: zerofrom::ZeroFrom<'static, M::Yokeable>,
    {
        DataPayload(DataPayloadInner::Yoke(
            match self.0 {
                DataPayloadInner::Yoke(yoke) => yoke,
                DataPayloadInner::StaticRef(r) => Yoke::new_owned(zerofrom::ZeroFrom::zero_from(r))
                    .convert_cart_into_option_pointer(),
            }
            .map_project(f),
        ))
    }

    /// Version of [`DataPayload::map_project()`] that borrows `self` instead of moving `self`.
    ///
    /// # Examples
    ///
    /// Same example as above, but this time, do not move out of `p1`:
    ///
    /// ```
    /// // Same imports and definitions as above
    /// # use icu_provider::hello_world::*;
    /// # use icu_provider::prelude::*;
    /// # use std::borrow::Cow;
    /// # struct HelloWorldV1MessageMarker;
    /// # impl DataMarker for HelloWorldV1MessageMarker {
    /// #     type Yokeable = Cow<'static, str>;
    /// # }
    ///
    /// let p1: DataPayload<HelloWorldV1Marker> =
    ///     DataPayload::from_owned(HelloWorldV1 {
    ///         message: Cow::Borrowed("Hello World"),
    ///     });
    ///
    /// assert_eq!("Hello World", p1.get().message);
    ///
    /// let p2: DataPayload<HelloWorldV1MessageMarker> =
    ///     p1.map_project_cloned(|obj, _| obj.message.clone());
    ///
    /// // Note: p1 is still valid.
    /// assert_eq!(p1.get().message, *p2.get());
    /// ```
    #[allow(clippy::type_complexity)]
    pub fn map_project_cloned<'this, M2, F>(&'this self, f: F) -> DataPayload<M2>
    where
        M2: DataMarker,
        F: for<'a> FnOnce(
            &'this <M::Yokeable as Yokeable<'a>>::Output,
            PhantomData<&'a ()>,
        ) -> <M2::Yokeable as Yokeable<'a>>::Output,
    {
        DataPayload(DataPayloadInner::Yoke(match &self.0 {
            DataPayloadInner::Yoke(yoke) => yoke.map_project_cloned(f),
            DataPayloadInner::StaticRef(r) => {
                let output: <M2::Yokeable as Yokeable<'static>>::Output =
                    f(Yokeable::transform(*r), PhantomData);
                // Safety: <M2::Yokeable as Yokeable<'static>>::Output is the same type as M2::Yokeable;
                // we're going from 'static to 'static, however in a generic context it's not
                // clear to the compiler that that is the case. We have to use the unsafe make API to do this.
                let yokeable: M2::Yokeable = unsafe { M2::Yokeable::make(output) };
                Yoke::new_owned(yokeable).convert_cart_into_option_pointer()
            }
        }))
    }

    /// Version of [`DataPayload::map_project()`] that bubbles up an error from `f`.
    ///
    /// # Examples
    ///
    /// Same example as above, but bubble up an error:
    ///
    /// ```
    /// // Same imports and definitions as above
    /// # use icu_provider::hello_world::*;
    /// # use icu_provider::prelude::*;
    /// # use std::borrow::Cow;
    /// # struct HelloWorldV1MessageMarker;
    /// # impl DataMarker for HelloWorldV1MessageMarker {
    /// #     type Yokeable = Cow<'static, str>;
    /// # }
    ///
    /// let p1: DataPayload<HelloWorldV1Marker> =
    ///     DataPayload::from_owned(HelloWorldV1 {
    ///         message: Cow::Borrowed("Hello World"),
    ///     });
    ///
    /// assert_eq!("Hello World", p1.get().message);
    ///
    /// let string_to_append = "Extra";
    /// let p2: DataPayload<HelloWorldV1MessageMarker> =
    ///     p1.try_map_project(|mut obj, _| {
    ///         if obj.message.is_empty() {
    ///             return Err("Example error");
    ///         }
    ///         obj.message.to_mut().push_str(string_to_append);
    ///         Ok(obj.message)
    ///     })?;
    ///
    /// assert_eq!("Hello WorldExtra", p2.get());
    /// # Ok::<(), &'static str>(())
    /// ```
    #[allow(clippy::type_complexity)]
    pub fn try_map_project<M2, F, E>(self, f: F) -> Result<DataPayload<M2>, E>
    where
        M2: DataMarker,
        F: for<'a> FnOnce(
            <M::Yokeable as Yokeable<'a>>::Output,
            PhantomData<&'a ()>,
        ) -> Result<<M2::Yokeable as Yokeable<'a>>::Output, E>,
        M::Yokeable: zerofrom::ZeroFrom<'static, M::Yokeable>,
    {
        Ok(DataPayload(DataPayloadInner::Yoke(
            match self.0 {
                DataPayloadInner::Yoke(yoke) => yoke,
                DataPayloadInner::StaticRef(r) => Yoke::new_owned(zerofrom::ZeroFrom::zero_from(r))
                    .convert_cart_into_option_pointer(),
            }
            .try_map_project(f)?,
        )))
    }

    /// Version of [`DataPayload::map_project_cloned()`] that  bubbles up an error from `f`.
    ///
    /// # Examples
    ///
    /// Same example as above, but bubble up an error:
    ///
    /// ```
    /// // Same imports and definitions as above
    /// # use icu_provider::hello_world::*;
    /// # use icu_provider::prelude::*;
    /// # use std::borrow::Cow;
    /// # struct HelloWorldV1MessageMarker;
    /// # impl DataMarker for HelloWorldV1MessageMarker {
    /// #     type Yokeable = Cow<'static, str>;
    /// # }
    ///
    /// let p1: DataPayload<HelloWorldV1Marker> =
    ///     DataPayload::from_owned(HelloWorldV1 {
    ///         message: Cow::Borrowed("Hello World"),
    ///     });
    ///
    /// assert_eq!("Hello World", p1.get().message);
    ///
    /// let string_to_append = "Extra";
    /// let p2: DataPayload<HelloWorldV1MessageMarker> = p1
    ///     .try_map_project_cloned(|obj, _| {
    ///         if obj.message.is_empty() {
    ///             return Err("Example error");
    ///         }
    ///         let mut message = obj.message.clone();
    ///         message.to_mut().push_str(string_to_append);
    ///         Ok(message)
    ///     })?;
    ///
    /// // Note: p1 is still valid, but the values no longer equal.
    /// assert_ne!(p1.get().message, *p2.get());
    /// assert_eq!("Hello WorldExtra", p2.get());
    /// # Ok::<(), &'static str>(())
    /// ```
    #[allow(clippy::type_complexity)]
    pub fn try_map_project_cloned<'this, M2, F, E>(&'this self, f: F) -> Result<DataPayload<M2>, E>
    where
        M2: DataMarker,
        F: for<'a> FnOnce(
            &'this <M::Yokeable as Yokeable<'a>>::Output,
            PhantomData<&'a ()>,
        ) -> Result<<M2::Yokeable as Yokeable<'a>>::Output, E>,
    {
        Ok(DataPayload(DataPayloadInner::Yoke(match &self.0 {
            DataPayloadInner::Yoke(yoke) => yoke.try_map_project_cloned(f)?,
            DataPayloadInner::StaticRef(r) => {
                let output: <M2::Yokeable as Yokeable<'static>>::Output =
                    f(Yokeable::transform(*r), PhantomData)?;
                // Safety: <M2::Yokeable as Yokeable<'static>>::Output is the same type as M2::Yokeable
                Yoke::new_owned(unsafe { M2::Yokeable::make(output) })
                    .convert_cart_into_option_pointer()
            }
        })))
    }

    /// Convert between two [`DataMarker`] types that are compatible with each other
    /// with compile-time type checking.
    ///
    /// This happens if they both have the same [`DataMarker::Yokeable`] type.
    ///
    /// Can be used to erase the key of a data payload in cases where multiple keys correspond
    /// to the same data struct.
    ///
    /// For runtime dynamic casting, use [`DataPayload::dynamic_cast_mut()`].
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use icu_provider::hello_world::*;
    /// use icu_provider::prelude::*;
    ///
    /// struct CustomHelloWorldV1Marker;
    /// impl DataMarker for CustomHelloWorldV1Marker {
    ///     type Yokeable = HelloWorldV1<'static>;
    /// }
    ///
    /// let hello_world: DataPayload<HelloWorldV1Marker> = todo!();
    /// let custom: DataPayload<CustomHelloWorldV1Marker> = hello_world.cast();
    /// ```
    #[inline]
    pub fn cast<M2>(self) -> DataPayload<M2>
    where
        M2: DataMarker<Yokeable = M::Yokeable>,
    {
        DataPayload(match self.0 {
            DataPayloadInner::Yoke(yoke) => DataPayloadInner::Yoke(yoke),
            DataPayloadInner::StaticRef(r) => DataPayloadInner::StaticRef(r),
        })
    }

    /// Convert a mutable reference of a [`DataPayload`] to another mutable reference
    /// of the same type with runtime type checking.
    ///
    /// Primarily useful to convert from a generic to a concrete marker type.
    ///
    /// If the `M2` type argument does not match the true marker type, a `DataError` is returned.
    ///
    /// For compile-time static casting, use [`DataPayload::cast()`].
    ///
    /// # Examples
    ///
    /// Change the results of a particular request based on key:
    ///
    /// ```
    /// use icu_locid::locale;
    /// use icu_provider::hello_world::*;
    /// use icu_provider::prelude::*;
    ///
    /// struct MyWrapper<P> {
    ///     inner: P,
    /// }
    ///
    /// impl<M, P> DataProvider<M> for MyWrapper<P>
    /// where
    ///     M: KeyedDataMarker,
    ///     P: DataProvider<M>,
    /// {
    ///     #[inline]
    ///     fn load(&self, req: DataRequest) -> Result<DataResponse<M>, DataError> {
    ///         let mut res = self.inner.load(req)?;
    ///         if let Some(ref mut generic_payload) = res.payload {
    ///             let mut cast_result =
    ///                 generic_payload.dynamic_cast_mut::<HelloWorldV1Marker>();
    ///             if let Ok(ref mut concrete_payload) = cast_result {
    ///                 // Add an emoji to the hello world message
    ///                 concrete_payload.with_mut(|data| {
    ///                     data.message.to_mut().insert_str(0, "✨ ");
    ///                 });
    ///             }
    ///         }
    ///         Ok(res)
    ///     }
    /// }
    ///
    /// let provider = MyWrapper {
    ///     inner: HelloWorldProvider,
    /// };
    /// let formatter =
    ///     HelloWorldFormatter::try_new_unstable(&provider, &locale!("de").into())
    ///         .unwrap();
    ///
    /// assert_eq!(formatter.format_to_string(), "✨ Hallo Welt");
    /// ```
    #[inline]
    pub fn dynamic_cast_mut<M2>(&mut self) -> Result<&mut DataPayload<M2>, DataError>
    where
        M2: DataMarker,
    {
        let this: &mut dyn core::any::Any = self;
        if let Some(this) = this.downcast_mut() {
            Ok(this)
        } else {
            Err(DataError::for_type::<M2>().with_str_context(core::any::type_name::<M>()))
        }
    }
}

impl DataPayload<BufferMarker> {
    /// Converts an owned byte buffer into a `DataPayload<BufferMarker>`.
    pub fn from_owned_buffer(buffer: Box<[u8]>) -> Self {
        let yoke = Yoke::attach_to_cart(SelectedRc::new(buffer), |b| &**b)
            .wrap_cart_in_option()
            .convert_cart_into_option_pointer();
        Self(DataPayloadInner::Yoke(yoke))
    }

    /// Converts a yoked byte buffer into a `DataPayload<BufferMarker>`.
    pub fn from_yoked_buffer(yoke: Yoke<&'static [u8], Option<Cart>>) -> Self {
        let yoke = Cart::unwrap_cart(yoke);
        Self(DataPayloadInner::Yoke(
            yoke.convert_cart_into_option_pointer(),
        ))
    }

    /// Converts a static byte buffer into a `DataPayload<BufferMarker>`.
    pub fn from_static_buffer(buffer: &'static [u8]) -> Self {
        Self(DataPayloadInner::Yoke(
            Yoke::new_owned(buffer).convert_cart_into_option_pointer(),
        ))
    }
}

impl<M> Default for DataPayload<M>
where
    M: DataMarker,
    M::Yokeable: Default,
{
    fn default() -> Self {
        Self::from_owned(Default::default())
    }
}

impl<M, O> DataPayloadOr<M, O>
where
    M: DataMarker,
{
    /// Creates a [`DataPayloadOr`] from a [`DataPayload`].
    #[inline]
    pub fn from_payload(payload: DataPayload<M>) -> Self {
        match payload.0 {
            DataPayloadInner::Yoke(yoke) => Self(DataPayloadOrInner::Yoke(yoke)),
            DataPayloadInner::StaticRef(r) => Self(DataPayloadOrInner::Inner(
                DataPayloadOrInnerInner::StaticRef(r),
            )),
        }
    }

    /// Creates a [`DataPayloadOr`] from the other type `O`.
    #[inline]
    pub fn from_other(other: O) -> Self {
        Self(DataPayloadOrInner::Inner(DataPayloadOrInnerInner::Other(
            other,
        )))
    }

    /// Gets the value from this [`DataPayload`] as `Ok` or the other type as `Err`.
    #[allow(clippy::needless_lifetimes)]
    #[inline]
    pub fn get<'a>(&'a self) -> Result<&'a <M::Yokeable as Yokeable<'a>>::Output, &'a O> {
        match &self.0 {
            DataPayloadOrInner::Yoke(yoke) => Ok(yoke.get()),
            DataPayloadOrInner::Inner(DataPayloadOrInnerInner::StaticRef(r)) => {
                Ok(Yokeable::transform(*r))
            }
            DataPayloadOrInner::Inner(DataPayloadOrInnerInner::Other(o)) => Err(o),
        }
    }

    /// Consumes this [`DataPayloadOr`], returning either the wrapped
    /// [`DataPayload`] or the other type.
    #[inline]
    pub fn into_inner(self) -> Result<DataPayload<M>, O> {
        match self.0 {
            DataPayloadOrInner::Yoke(yoke) => Ok(DataPayload(DataPayloadInner::Yoke(yoke))),
            DataPayloadOrInner::Inner(DataPayloadOrInnerInner::StaticRef(r)) => {
                Ok(DataPayload(DataPayloadInner::StaticRef(r)))
            }
            DataPayloadOrInner::Inner(DataPayloadOrInnerInner::Other(o)) => Err(o),
        }
    }
}

/// A response object containing an object as payload and metadata about it.
#[allow(clippy::exhaustive_structs)] // this type is stable
pub struct DataResponse<M>
where
    M: DataMarker,
{
    /// Metadata about the returned object.
    pub metadata: DataResponseMetadata,

    /// The object itself; `None` if it was not loaded.
    pub payload: Option<DataPayload<M>>,
}

impl<M> DataResponse<M>
where
    M: DataMarker,
{
    /// Takes ownership of the underlying payload. Error if not present.
    ///
    /// To take the metadata, too, use [`Self::take_metadata_and_payload()`].
    #[inline]
    pub fn take_payload(self) -> Result<DataPayload<M>, DataError> {
        Ok(self.take_metadata_and_payload()?.1)
    }

    /// Takes ownership of the underlying metadata and payload. Error if payload is not present.
    #[inline]
    pub fn take_metadata_and_payload(
        self,
    ) -> Result<(DataResponseMetadata, DataPayload<M>), DataError> {
        Ok((
            self.metadata,
            self.payload
                .ok_or_else(|| DataErrorKind::MissingPayload.with_type_context::<M>())?,
        ))
    }

    /// Convert between two [`DataMarker`] types that are compatible with each other
    /// with compile-time type checking.
    ///
    /// This happens if they both have the same [`DataMarker::Yokeable`] type.
    ///
    /// Can be used to erase the key of a data payload in cases where multiple keys correspond
    /// to the same data struct.
    ///
    /// For runtime dynamic casting, use [`DataPayload::dynamic_cast_mut()`].
    #[inline]
    pub fn cast<M2>(self) -> DataResponse<M2>
    where
        M2: DataMarker<Yokeable = M::Yokeable>,
    {
        match self.payload {
            Some(payload) => DataResponse {
                metadata: self.metadata,
                payload: Some(payload.cast()),
            },
            None => DataResponse {
                metadata: self.metadata,
                payload: None,
            },
        }
    }
}

impl<M> TryFrom<DataResponse<M>> for DataPayload<M>
where
    M: DataMarker,
{
    type Error = DataError;

    fn try_from(response: DataResponse<M>) -> Result<Self, Self::Error> {
        response.take_payload()
    }
}

impl<M> Debug for DataResponse<M>
where
    M: DataMarker,
    for<'a> &'a <M::Yokeable as Yokeable<'a>>::Output: Debug,
{
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "DataResponse {{ metadata: {:?}, payload: {:?} }}",
            self.metadata, self.payload
        )
    }
}

/// Cloning a DataResponse is generally a cheap operation.
/// See notes in the `Clone` impl for [`Yoke`].
///
/// # Examples
///
/// ```no_run
/// use icu_provider::hello_world::*;
/// use icu_provider::prelude::*;
///
/// let resp1: DataResponse<HelloWorldV1Marker> = todo!();
/// let resp2 = resp1.clone();
/// ```
impl<M> Clone for DataResponse<M>
where
    M: DataMarker,
    for<'a> YokeTraitHack<<M::Yokeable as Yokeable<'a>>::Output>: Clone,
{
    fn clone(&self) -> Self {
        Self {
            metadata: self.metadata.clone(),
            payload: self.payload.clone(),
        }
    }
}

#[test]
fn test_debug() {
    use crate::hello_world::*;
    use alloc::borrow::Cow;
    let resp = DataResponse::<HelloWorldV1Marker> {
        metadata: Default::default(),
        payload: Some(DataPayload::from_owned(HelloWorldV1 {
            message: Cow::Borrowed("foo"),
        })),
    };
    assert_eq!("DataResponse { metadata: DataResponseMetadata { locale: None, buffer_format: None }, payload: Some(HelloWorldV1 { message: \"foo\" }) }", format!("{resp:?}"));
}
