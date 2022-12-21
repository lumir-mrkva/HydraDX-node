#![allow(warnings)]
// This file is part of pallet-dca.

// Copyright (C) 2020-2022  Intergalactic, Limited (GIB).
// SPDX-License-Identifier: Apache-2.0

// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
#![cfg_attr(not(feature = "std"), no_std)]

use codec::{Decode, Encode, MaxEncodedLen};
use frame_support::ensure;
use frame_support::pallet_prelude::*;
use frame_support::traits::fungibles::Inspect;
use frame_support::traits::Get;
use frame_support::transactional;
use frame_system::ensure_signed;
use frame_system::pallet_prelude::OriginFor;
use frame_system::Origin;
use orml_traits::arithmetic::{CheckedAdd, CheckedSub};
use scale_info::TypeInfo;
use sp_runtime::traits::Saturating;
use sp_runtime::traits::Zero;
use sp_runtime::traits::{BlockNumberProvider, ConstU32};
use sp_runtime::ArithmeticError;
use sp_runtime::{BoundedVec, DispatchError};
use sp_std::vec::Vec;
#[cfg(test)]
mod tests;

pub mod types;
pub mod weights;

use weights::WeightInfo;

// Re-export pallet items so that they can be accessed from the crate namespace.
use crate::types::{AssetId, Balance, BlockNumber, ScheduleId};
pub use pallet::*;

const MAX_NUMBER_OF_TRADES: u32 = 5;
const MAX_NUMBER_OF_SCHEDULES_PER_BLOCK: u32 = 20; //TODO: use config for this

type BlockNumberFor<T> = <T as frame_system::Config>::BlockNumber;

#[derive(Encode, Decode, Debug, Eq, PartialEq, Clone, TypeInfo, MaxEncodedLen)]
pub enum Recurrence {
	Fixed(u128),
	Perpetual,
}

#[derive(Encode, Decode, Debug, Eq, PartialEq, Clone, TypeInfo, MaxEncodedLen)]
pub struct Order<AssetId> {
	pub asset_in: AssetId,
	pub asset_out: AssetId,
	pub amount_in: Balance,
	pub amount_out: Balance,
	pub limit: Balance,
	pub route: BoundedVec<Trade, sp_runtime::traits::ConstU32<MAX_NUMBER_OF_TRADES>>,
}

#[derive(Encode, Decode, Debug, Eq, PartialEq, Clone, TypeInfo, MaxEncodedLen)]
pub struct Schedule<AssetId> {
	pub period: BlockNumber, //TODO: use proper block number
	pub recurrence: Recurrence,
	pub order: Order<AssetId>,
}

///A single trade for buy/sell, describing the asset pair and the pool type in which the trade is executed
#[derive(Encode, Decode, Debug, Eq, PartialEq, Clone, TypeInfo, MaxEncodedLen)]
pub struct Trade {
	pub pool: PoolType, //TODO: consider using the same type as in route executor
	pub asset_in: AssetId,
	pub asset_out: AssetId,
}

#[derive(Encode, Decode, Clone, Copy, Debug, Eq, PartialEq, TypeInfo, MaxEncodedLen)]
pub enum PoolType {
	XYK,
}

#[derive(Encode, Decode, Debug, Eq, PartialEq, Clone, TypeInfo, MaxEncodedLen)]
pub struct Bond {
	pub asset: AssetId,
	pub amount: Balance,
}

#[frame_support::pallet]
pub mod pallet {
	use super::*;
	use codec::{EncodeLike, HasCompact};
	use frame_system::pallet_prelude::OriginFor;
	use hydradx_traits::router::ExecutorError;
	use sp_runtime::traits::{MaybeDisplay, Saturating};
	use std::fmt::Debug;

	#[pallet::pallet]
	#[pallet::generate_store(pub(super) trait Store)]
	pub struct Pallet<T>(_);

	#[pallet::hooks]
	impl<T: Config> Hooks<T::BlockNumber> for Pallet<T>
	where
		<T as pallet_omnipool::Config>::AssetId: From<<T as pallet::Config>::Asset>,
	{
		fn on_initialize(current_blocknumber: T::BlockNumber) -> Weight {
			{
				let mut weight: u64 = 0;

				let schedules: BoundedVec<ScheduleId, ConstU32<20>> =
					ScheduleIdsPerBlock::<T>::get(current_blocknumber).unwrap(); //TODO: better error handling for all the unwrap

				for schedule_id in schedules {
					let schedule = Schedules::<T>::get(schedule_id).unwrap();
					let owner = ScheduleOwnership::<T>::get(schedule_id).unwrap();
					let origin: OriginFor<T> = Origin::<T>::Signed(owner).into();

					let buy_result = Self::execute_buy(origin, &schedule);

					match buy_result {
						Ok(res) => {
							let blocknumber_for_schedule =
								current_blocknumber.checked_add(&schedule.period.into()).unwrap();

							match schedule.recurrence {
								Recurrence::Fixed(_) => {
									let remaining_reccurences = Self::decrement_recurrences(schedule_id).unwrap();
									if !remaining_reccurences.is_zero() {
										Self::plan_schedule_for_block(blocknumber_for_schedule, schedule_id, &schedule);
									}
								}
								Recurrence::Perpetual => {
									Self::plan_schedule_for_block(blocknumber_for_schedule, schedule_id, &schedule);
								}
							}
						}
						_ => {
							//Suspend
						}
					}
				}

				//TODO: increment the weight once an action happens
				//weight += T::WeightInfo::get_spot_price().ref_time();

				Weight::from_ref_time(weight)
			}
		}
	}

	#[pallet::config]
	pub trait Config: frame_system::Config + pallet_omnipool::Config {
		type Event: From<Event<Self>> + IsType<<Self as frame_system::Config>::Event>;

		/// Identifier for the class of asset.
		type Asset: Member
			+ Parameter
			+ Default
			+ Copy
			+ HasCompact
			+ MaybeSerializeDeserialize
			+ MaxEncodedLen
			+ TypeInfo;

		/// Weight information for the extrinsics.
		type WeightInfo: WeightInfo;
	}

	#[pallet::event]
	#[pallet::generate_deposit(pub(crate) fn deposit_event)]
	pub enum Event<T: Config> {
		///First event
		DummyEvent {},
	}

	#[pallet::error]
	pub enum Error<T> {
		///First error
		UnexpectedError,
	}

	/// Id sequencer for schedules
	#[pallet::storage]
	#[pallet::getter(fn next_schedule_id)]
	pub type ScheduleIdSequencer<T: Config> = StorageValue<_, ScheduleId, ValueQuery>;

	#[pallet::storage]
	#[pallet::getter(fn schedules)]
	pub type Schedules<T: Config> = StorageMap<_, Blake2_128Concat, ScheduleId, Schedule<T::Asset>, OptionQuery>;

	#[pallet::storage]
	#[pallet::getter(fn schedule_ownership)]
	pub type ScheduleOwnership<T: Config> = StorageMap<_, Blake2_128Concat, ScheduleId, T::AccountId, OptionQuery>;

	//TODO: the number of recurrences can not be 0, so remove item once there - https://www.notion.so/DCA-061a93f912fd43b3a8e3e413abb8afdf#24dc5396cce542f681862ec7b1e54c15
	#[pallet::storage]
	#[pallet::getter(fn remaining_recurrences)]
	pub type RemainingRecurrences<T: Config> = StorageMap<_, Blake2_128Concat, ScheduleId, u128, OptionQuery>;

	#[pallet::storage]
	#[pallet::getter(fn schedule_ids_per_block)]
	pub type ScheduleIdsPerBlock<T: Config> =
		StorageMap<_, Blake2_128Concat, BlockNumberFor<T>, BoundedVec<ScheduleId, ConstU32<20>>, OptionQuery>;

	#[pallet::call]
	impl<T: Config> Pallet<T>
	where
		<T as pallet_omnipool::Config>::AssetId: From<<T as pallet::Config>::Asset>,
	{
		///Schedule
		#[pallet::weight(<T as Config>::WeightInfo::sell(5))]
		#[transactional]
		pub fn schedule(
			origin: OriginFor<T>,
			schedule: Schedule<T::Asset>,
			next_execution_block: Option<BlockNumberFor<T>>,
		) -> DispatchResult {
			let who = ensure_signed(origin.clone())?;

			let next_schedule_id = Self::get_next_schedule_id()?;

			Schedules::<T>::insert(next_schedule_id, &schedule);
			Self::store_recurrence_in_case_of_fixed_schedule(next_schedule_id, &schedule.recurrence);
			ScheduleOwnership::<T>::insert(next_schedule_id, who);

			let blocknumber_for_schedule = next_execution_block.unwrap_or_else(|| Self::get_next_block_mumber());
			Self::plan_schedule_for_block(blocknumber_for_schedule, next_schedule_id, &schedule);

			//TODO: emit events

			Ok(())
		}
	}
}

impl<T: Config> Pallet<T> {
	fn plan_schedule_for_block(b: T::BlockNumber, schedule_id: ScheduleId, schedule: &Schedule<<T as Config>::Asset>) {
		if !ScheduleIdsPerBlock::<T>::contains_key(b) {
			let vec_with_first_schedule_id = Self::create_bounded_vec(schedule_id);
			ScheduleIdsPerBlock::<T>::insert(b, vec_with_first_schedule_id);
		} else {
			//TODO: if the block is full, then we should plan it the next one or so
			Self::add_schedule_id_to_existing_ids_per_block(schedule_id, b);
		}
	}

	fn get_next_schedule_id() -> Result<ScheduleId, ArithmeticError> {
		ScheduleIdSequencer::<T>::try_mutate(|current_id| {
			*current_id = current_id.checked_add(1).ok_or(ArithmeticError::Overflow)?;

			Ok(*current_id)
		})
	}

	fn get_next_block_mumber() -> BlockNumberFor<T> {
		let mut current_block_number = frame_system::Pallet::<T>::current_block_number();
		current_block_number.saturating_inc();

		current_block_number
	}

	fn create_bounded_vec(next_schedule_id: ScheduleId) -> BoundedVec<ScheduleId, ConstU32<20>> {
		let schedule_id = vec![next_schedule_id];
		let bounded_vec: BoundedVec<ScheduleId, ConstU32<20>> = schedule_id.try_into().unwrap(); //TODO: here use constant instead of hardcoded value
		bounded_vec
	}

	fn store_recurrence_in_case_of_fixed_schedule(next_schedule_id: ScheduleId, recurrence: &Recurrence) {
		if let Recurrence::Fixed(number_of_recurrence) = *recurrence {
			RemainingRecurrences::<T>::insert(next_schedule_id, number_of_recurrence);
		};
	}

	fn add_schedule_id_to_existing_ids_per_block(
		next_schedule_id: ScheduleId,
		blocknumber_for_schedule: <T as frame_system::Config>::BlockNumber,
	) -> DispatchResult {
		ScheduleIdsPerBlock::<T>::try_mutate_exists(blocknumber_for_schedule, |schedule_ids| -> DispatchResult {
			let mut schedule_ids = schedule_ids.as_mut().ok_or(Error::<T>::UnexpectedError)?; //TODO: add different error handling

			schedule_ids
				.try_push(next_schedule_id)
				.map_err(|_| Error::<T>::UnexpectedError)?;
			Ok(())
		})?;

		Ok(())
	}

	fn decrement_recurrences(schedule_id: ScheduleId) -> Result<u128, DispatchResult> {
		let remaining_recurrences =
			RemainingRecurrences::<T>::try_mutate_exists(schedule_id, |maybe_remaining_occurrances| {
				let mut remaining_ocurrences = maybe_remaining_occurrances
					.as_mut()
					.ok_or(Error::<T>::UnexpectedError)?; //TODO: add RaminingReccurenceNotExist error

				*remaining_ocurrences = remaining_ocurrences.checked_sub(1).ok_or(Error::<T>::UnexpectedError)?; //TODO: add arithmetic error
				let remainings = remaining_ocurrences.clone(); //TODO: do this in a smarter way?

				if *remaining_ocurrences == 0 {
					*maybe_remaining_occurrances = None;
				}

				Ok::<u128, DispatchError>(remainings)
			})?;

		Ok(remaining_recurrences)
	}

	fn execute_buy(origin: T::Origin, schedule: &Schedule<<T as Config>::Asset>) -> DispatchResult
	where
		<T as pallet_omnipool::Config>::AssetId: From<<T as pallet::Config>::Asset>,
	{
		let buy_result = pallet_omnipool::Pallet::<T>::buy(
			origin,
			schedule.order.asset_out.into(),
			schedule.order.asset_in.into(),
			schedule.order.amount_out,
			schedule.order.limit,
		);
		buy_result
	}
}
