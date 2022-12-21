use serde::de::DeserializeOwned;
use serde::Serialize;
use std::marker::PhantomData;

use cosmwasm_std::{
    to_vec, Addr, CustomQuery, QuerierWrapper, StdError, StdResult, Storage, WasmQuery,
};

use crate::helpers::{may_deserialize, must_deserialize};

/// Item stores one typed item at the given key.
/// This is an analog of Singleton.
/// It functions the same way as Path does but doesn't use a Vec and thus has a const fn constructor.
pub struct Item<'a, T> {
    // this is full key - no need to length-prefix it, we only store one item
    storage_key: &'a [u8],
    // see https://doc.rust-lang.org/std/marker/struct.PhantomData.html#unused-type-parameters for why this is needed
    data_type: PhantomData<T>,
}

impl<'a, T> Item<'a, T> {
    pub const fn new(storage_key: &'a str) -> Self {
        Item {
            storage_key: storage_key.as_bytes(),
            data_type: PhantomData,
        }
    }
}

impl<'a, T> Item<'a, T>
where
    T: Serialize + DeserializeOwned,
{
    // this gets the path of the data to use elsewhere
    pub fn as_slice(&self) -> &[u8] {
        self.storage_key
    }

    /// save will serialize the model and store, returns an error on serialization issues
    pub fn save(&self, store: &mut dyn Storage, data: &T) -> StdResult<()> {
        store.set(self.storage_key, &to_vec(data)?);
        Ok(())
    }

    pub fn remove(&self, store: &mut dyn Storage) {
        store.remove(self.storage_key);
    }

    /// load will return an error if no data is set at the given key, or on parse error
    pub fn load(&self, store: &dyn Storage) -> StdResult<T> {
        let value = store.get(self.storage_key);
        must_deserialize(&value)
    }

    /// may_load will parse the data stored at the key if present, returns `Ok(None)` if no data there.
    /// returns an error on issues parsing
    pub fn may_load(&self, store: &dyn Storage) -> StdResult<Option<T>> {
        let value = store.get(self.storage_key);
        may_deserialize(&value)
    }

    /// Loads the data, perform the specified action, and store the result
    /// in the database. This is shorthand for some common sequences, which may be useful.
    ///
    /// It assumes, that data was initialized before, and if it doesn't exist, `Err(StdError::NotFound)`
    /// is returned.
    pub fn update<A, E>(&self, store: &mut dyn Storage, action: A) -> Result<T, E>
    where
        A: FnOnce(T) -> Result<T, E>,
        E: From<StdError>,
    {
        let input = self.load(store)?;
        let output = action(input)?;
        self.save(store, &output)?;
        Ok(output)
    }

    /// If you import the proper Item from the remote contract, this will let you read the data
    /// from a remote contract in a type-safe way using WasmQuery::RawQuery.
    ///
    /// Note that we expect an Item to be set, and error if there is no data there
    pub fn query<Q: CustomQuery>(
        &self,
        querier: &QuerierWrapper,
        remote_contract: Addr,
    ) -> StdResult<T> {
        let request = WasmQuery::Raw {
            contract_addr: remote_contract.into(),
            key: self.storage_key.into(),
        };
        querier.query(&request.into())
    }
}
