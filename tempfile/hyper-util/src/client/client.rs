use hyper::{Request, Response};
use tower::{Service, MakeService};

use super::connect::Connect;
use super::pool;

pub struct Client<M> {
    // Hi there. So, let's take a 0.14.x hyper::Client, and build up its layers
    // here. We don't need to fully expose the layers to start with, but that
    // is the end goal.
    //
    // Client = MakeSvcAsService<
    //   SetHost<
    //     Http1RequestTarget<
    //       DelayedRelease<
    //         ConnectingPool<C, P>
    //       >
    //     >
    //   >
    // >
    make_svc: M,
}

// We might change this... :shrug:
type PoolKey = hyper::Uri;

struct ConnectingPool<C, P> {
    connector: C,
    pool: P,
}

struct PoolableSvc<S>(S);

/// A marker to identify what version a pooled connection is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[allow(dead_code)]
pub enum Ver {
    Auto,
    Http2,
}

// ===== impl Client =====

impl<M, /*ReqBody, ResBody,*/ E> Client<M>
where
    M: MakeService<
        hyper::Uri,
        Request<()>,
        Response = Response<()>,
        Error = E,
        MakeError = E,
    >,
    //M: Service<hyper::Uri, Error = E>,
    //M::Response: Service<Request<ReqBody>, Response = Response<ResBody>>,
{
    pub async fn request(&mut self, req: Request<()>) -> Result<Response<()>, E> {
        let mut svc = self.make_svc.make_service(req.uri().clone()).await?;
        svc.call(req).await
    }
}

impl<M, /*ReqBody, ResBody,*/ E> Client<M>
where
    M: MakeService<
        hyper::Uri,
        Request<()>,
        Response = Response<()>,
        Error = E,
        MakeError = E,
    >,
    //M: Service<hyper::Uri, Error = E>,
    //M::Response: Service<Request<ReqBody>, Response = Response<ResBody>>,
{
    
}

// ===== impl ConnectingPool =====

impl<C, P> ConnectingPool<C, P>
where
    C: Connect,
    C::_Svc: Unpin + Send + 'static,
{
    async fn connection_for(&self, target: PoolKey) -> Result<pool::Pooled<PoolableSvc<C::_Svc>, PoolKey>, ()> {
        todo!()
    }
}

impl<S> pool::Poolable for PoolableSvc<S>
where
    S: Unpin + Send + 'static,
{
    fn is_open(&self) -> bool {
        /*
        match self.tx {
            PoolTx::Http1(ref tx) => tx.is_ready(),
            #[cfg(feature = "http2")]
            PoolTx::Http2(ref tx) => tx.is_ready(),
        }
        */
        true
    }

    fn reserve(self) -> pool::Reservation<Self> {
        /*
        match self.tx {
            PoolTx::Http1(tx) => Reservation::Unique(PoolClient {
                conn_info: self.conn_info,
                tx: PoolTx::Http1(tx),
            }),
            #[cfg(feature = "http2")]
            PoolTx::Http2(tx) => {
                let b = PoolClient {
                    conn_info: self.conn_info.clone(),
                    tx: PoolTx::Http2(tx.clone()),
                };
                let a = PoolClient {
                    conn_info: self.conn_info,
                    tx: PoolTx::Http2(tx),
                };
                Reservation::Shared(a, b)
            }
        }
        */
        pool::Reservation::Unique(self)
    }

    fn can_share(&self) -> bool {
        false
        //self.is_http2()
    }
}
