struct ConnectingPool<C, P> {
    connector: C,
    pool: P,
}

struct PoolableSvc<S>(S);


