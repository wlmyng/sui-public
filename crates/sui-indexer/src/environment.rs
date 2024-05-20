use once_cell::sync::Lazy;

macro_rules! env {
    ($($name:ident: $ty:ty;)*) => {
        $(
            pub static $name: Lazy<Option<$ty>> = Lazy::new(|| {
                std::env::var(stringify!($name))
                    .ok()
                    .and_then(|s| s.parse().ok())
            });
        )*
    };
}

//*************************************************************************************************#
// DB
//*************************************************************************************************#

env! {
    DB_POOL_SIZE: u32;
    DB_CONNECTION_TIMEOUT: u64;
    DB_STATEMENT_TIMEOUT: u64;
}
