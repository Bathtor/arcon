// Copyright (c) 2020, KTH Royal Institute of Technology.
// SPDX-License-Identifier: AGPL-3.0-only

use crate::{
    prelude::ArconResult,
    state_backend::{
        serialization::{DeserializableWith, SerializableFixedSizeWith, SerializableWith},
        sled::{Sled, StateCommon},
        state_types::{State, ValueState},
    },
};
use std::marker::PhantomData;

pub struct SledValueState<IK, N, T, KS, TS> {
    pub(crate) common: StateCommon<IK, N, KS, TS>,
    pub(crate) _phantom: PhantomData<T>,
}

impl<IK, N, T, KS, TS> State<Sled, IK, N> for SledValueState<IK, N, T, KS, TS>
where
    IK: SerializableFixedSizeWith<KS>,
    N: SerializableFixedSizeWith<KS>,
    T: SerializableWith<TS>,
{
    fn clear(&self, backend: &mut Sled) -> ArconResult<()> {
        let key = self.common.get_db_key_prefix()?;
        backend.remove(&self.common.tree_name, &key)?;
        Ok(())
    }

    delegate_key_and_namespace!(common);
}

impl<IK, N, T, KS, TS> ValueState<Sled, IK, N, T> for SledValueState<IK, N, T, KS, TS>
where
    IK: SerializableFixedSizeWith<KS>,
    N: SerializableFixedSizeWith<KS>,
    T: SerializableWith<TS> + DeserializableWith<TS>,
{
    fn get(&self, backend: &Sled) -> ArconResult<Option<T>> {
        let key = self.common.get_db_key_prefix()?;
        if let Some(serialized) = backend.get(&self.common.tree_name, &key)? {
            let value = T::deserialize(&self.common.value_serializer, &serialized)?;
            Ok(Some(value))
        } else {
            Ok(None)
        }
    }

    fn set(&self, backend: &mut Sled, new_value: T) -> ArconResult<()> {
        let key = self.common.get_db_key_prefix()?;
        let serialized = T::serialize(&self.common.value_serializer, &new_value)?;
        backend.put(&self.common.tree_name, &key, &serialized)?;
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::state_backend::{
        serialization::NativeEndianBytesDump, sled::test::TestDb, ValueStateBuilder,
    };

    #[test]
    fn sled_value_state_test() {
        let mut db = TestDb::new();
        let value_state = db.new_value_state(
            "test_state",
            (),
            (),
            NativeEndianBytesDump,
            NativeEndianBytesDump,
        );

        let unset = value_state.get(&db).unwrap();
        assert!(unset.is_none());

        value_state.set(&mut db, 123).unwrap();
        let set = value_state.get(&db).unwrap().unwrap();
        assert_eq!(set, 123);

        value_state.clear(&mut db).unwrap();
        let cleared = value_state.get(&db).unwrap();
        assert!(cleared.is_none());
    }

    #[test]
    fn sled_value_states_are_independant() {
        let mut db = TestDb::new();
        let v1 = db.new_value_state(
            "test1",
            (),
            (),
            NativeEndianBytesDump,
            NativeEndianBytesDump,
        );
        let v2 = db.new_value_state(
            "test2",
            (),
            (),
            NativeEndianBytesDump,
            NativeEndianBytesDump,
        );

        v1.set(&mut db, 123).unwrap();
        v2.set(&mut db, 456).unwrap();

        let v1v = v1.get(&db).unwrap().unwrap();
        let v2v = v2.get(&db).unwrap().unwrap();
        assert_eq!(v1v, 123);
        assert_eq!(v2v, 456);

        v1.clear(&mut db).unwrap();
        let v1opt = v1.get(&db).unwrap();
        let v2v = v2.get(&db).unwrap().unwrap();
        assert!(v1opt.is_none());
        assert_eq!(v2v, 456);
    }

    #[test]
    fn sled_value_states_handle_state_for_different_keys_and_namespaces() {
        let mut db = TestDb::new();
        let mut value_state = db.new_value_state(
            "test_state",
            0,
            0,
            NativeEndianBytesDump,
            NativeEndianBytesDump,
        );

        value_state.set(&mut db, 0).unwrap();
        value_state.set_current_key(1).unwrap();
        let should_be_none = value_state.get(&db).unwrap();
        assert!(should_be_none.is_none());

        value_state.set(&mut db, 1).unwrap();
        let should_be_one = value_state.get(&db).unwrap().unwrap();
        assert_eq!(should_be_one, 1);

        value_state.set_current_key(0).unwrap();
        let should_be_zero = value_state.get(&db).unwrap().unwrap();
        assert_eq!(should_be_zero, 0);

        value_state.set_current_namespace(1).unwrap();
        let should_be_none = value_state.get(&db).unwrap();
        assert!(should_be_none.is_none());

        value_state.set(&mut db, 2).unwrap();
        let should_be_two = value_state.get(&db).unwrap().unwrap();
        assert_eq!(should_be_two, 2);

        value_state.set_current_namespace(0).unwrap();
        let should_be_zero = value_state.get(&db).unwrap().unwrap();
        assert_eq!(should_be_zero, 0);
    }
}
