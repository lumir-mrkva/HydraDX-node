//                    :                     $$\   $$\                 $$\                    $$$$$$$\  $$\   $$\
//                  !YJJ^                   $$ |  $$ |                $$ |                   $$  __$$\ $$ |  $$ |
//                7B5. ~B5^                 $$ |  $$ |$$\   $$\  $$$$$$$ | $$$$$$\  $$$$$$\  $$ |  $$ |\$$\ $$  |
//             .?B@G    ~@@P~               $$$$$$$$ |$$ |  $$ |$$  __$$ |$$  __$$\ \____$$\ $$ |  $$ | \$$$$  /
//           :?#@@@Y    .&@@@P!.            $$  __$$ |$$ |  $$ |$$ /  $$ |$$ |  \__|$$$$$$$ |$$ |  $$ | $$  $$<
//         ^?J^7P&@@!  .5@@#Y~!J!.          $$ |  $$ |$$ |  $$ |$$ |  $$ |$$ |     $$  __$$ |$$ |  $$ |$$  /\$$\
//       ^JJ!.   :!J5^ ?5?^    ^?Y7.        $$ |  $$ |\$$$$$$$ |\$$$$$$$ |$$ |     \$$$$$$$ |$$$$$$$  |$$ /  $$ |
//     ~PP: 7#B5!.         :?P#G: 7G?.      \__|  \__| \____$$ | \_______|\__|      \_______|\_______/ \__|  \__|
//  .!P@G    7@@@#Y^    .!P@@@#.   ~@&J:              $$\   $$ |
//  !&@@J    :&@@@@P.   !&@@@@5     #@@P.             \$$$$$$  |
//   :J##:   Y@@&P!      :JB@@&~   ?@G!                \______/
//     .?P!.?GY7:   .. .    ^?PP^:JP~
//       .7Y7.  .!YGP^ ?BP?^   ^JJ^         This file is part of https://github.com/galacticcouncil/HydraDX-node
//         .!Y7Y#@@#:   ?@@@G?JJ^           Built with <3 for decentralisation.
//            !G@@@Y    .&@@&J:
//              ^5@#.   7@#?.               Copyright (C) 2021-2023  Intergalactic, Limited (GIB).
//                :5P^.?G7.                 SPDX-License-Identifier: Apache-2.0
//                  :?Y!                    Licensed under the Apache License, Version 2.0 (the "License");
//                                          you may not use this file except in compliance with the License.
//                                          http://www.apache.org/licenses/LICENSE-2.0

use frame_support::log;

use crate::{
	evm::{
		precompiles::{
			erc20_mapping::{Erc20Mapping, HydraErc20Mapping},
			handle::{EvmDataWriter, FunctionModifier, PrecompileHandleExt},
			substrate::RuntimeHelper,
			succeed, Address, Output,
		},
		ExtendedAddressMapping,
	},
	Currencies,
};
use codec::EncodeLike;
use frame_support::traits::OriginTrait;
use hydradx_traits::InspectRegistry;
use num_enum::{IntoPrimitive, TryFromPrimitive};
use orml_traits::{MultiCurrency as MultiCurrencyT, MultiCurrency};
use pallet_evm::{AddressMapping, ExitRevert, Precompile, PrecompileFailure, PrecompileHandle, PrecompileResult};
use primitive_types::H160;
use primitives::{AssetId, Balance};
use sp_runtime::{traits::Dispatchable, RuntimeDebug};
use sp_std::{marker::PhantomData, prelude::*};

#[module_evm_utility_macro::generate_function_selector]
#[derive(RuntimeDebug, Eq, PartialEq, TryFromPrimitive, IntoPrimitive)]
#[repr(u32)]
pub enum Action {
	Name = "name()",
	Symbol = "symbol()",
	Decimals = "decimals()",
	TotalSupply = "totalSupply()",
	BalanceOf = "balanceOf(address)",
	Allowance = "allowance(address,address)",
	Transfer = "transfer(address,uint256)",
	Approve = "approve(address,uint256)",
	TransferFrom = "transferFrom(address,address,uint256)",
}
pub struct MultiCurrencyPrecompile<Runtime>(PhantomData<Runtime>);

impl<Runtime> Precompile for MultiCurrencyPrecompile<Runtime>
where
	Runtime: frame_system::Config + pallet_evm::Config + pallet_asset_registry::Config + pallet_currencies::Config,
	AssetId: EncodeLike<<Runtime as pallet_asset_registry::Config>::AssetId>,
	<<Runtime as frame_system::Config>::RuntimeCall as Dispatchable>::RuntimeOrigin: OriginTrait,
	<Runtime as pallet_asset_registry::Config>::AssetId: core::convert::From<AssetId>,
	Currencies: MultiCurrency<Runtime::AccountId, CurrencyId = AssetId, Balance = Balance>,
	pallet_currencies::Pallet<Runtime>: MultiCurrency<Runtime::AccountId, CurrencyId = AssetId, Balance = Balance>,
	<Runtime as frame_system::Config>::AccountId: core::convert::From<sp_runtime::AccountId32>,
	<<Runtime as frame_system::Config>::RuntimeCall as Dispatchable>::RuntimeOrigin: OriginTrait,
{
	fn execute(handle: &mut impl PrecompileHandle) -> pallet_evm::PrecompileResult {
		let address = handle.code_address();
		if let Some(asset_id) = HydraErc20Mapping::decode_evm_address(address) {
			log::debug!(target: "evm", "multicurrency: currency id: {:?}", asset_id);

			let selector = match handle.read_selector() {
				Ok(selector) => selector,
				Err(e) => return Err(e),
			};

			handle.check_function_modifier(match selector {
				Action::Transfer => FunctionModifier::NonPayable,
				_ => FunctionModifier::View,
			})?;

			return match selector {
				Action::Name => Self::name(asset_id, handle),
				Action::Symbol => Self::symbol(asset_id, handle),
				Action::Decimals => Self::decimals(asset_id, handle),
				Action::TotalSupply => Self::total_supply(asset_id, handle),
				Action::BalanceOf => Self::balance_of(asset_id, handle),
				Action::Transfer => Self::transfer(asset_id, handle),
				Action::Allowance => Self::not_supported(asset_id, handle),
				Action::Approve => Self::not_supported(asset_id, handle),
				Action::TransferFrom => Self::not_supported(asset_id, handle),
			};
		}
		Err(PrecompileFailure::Revert {
			exit_status: ExitRevert::Reverted,
			output: "invalid currency id".into(),
		})
	}
}

impl<Runtime> MultiCurrencyPrecompile<Runtime>
where
	Runtime: frame_system::Config + pallet_evm::Config + pallet_asset_registry::Config + pallet_currencies::Config,
	AssetId: EncodeLike<<Runtime as pallet_asset_registry::Config>::AssetId>,
	<Runtime as pallet_asset_registry::Config>::AssetId: core::convert::From<AssetId>,
	Currencies: MultiCurrency<Runtime::AccountId, CurrencyId = AssetId, Balance = Balance>,
	pallet_currencies::Pallet<Runtime>: MultiCurrency<Runtime::AccountId, CurrencyId = AssetId, Balance = Balance>,
	<Runtime as frame_system::Config>::AccountId: core::convert::From<sp_runtime::AccountId32>,
	<<Runtime as frame_system::Config>::RuntimeCall as Dispatchable>::RuntimeOrigin: OriginTrait,
{
	fn name(asset_id: AssetId, handle: &mut impl PrecompileHandle) -> PrecompileResult {
		handle.record_cost(RuntimeHelper::<Runtime>::db_read_gas_cost())?;

		// Parse input
		let input = handle.read_input()?;
		input.expect_arguments(0)?;

		match <pallet_asset_registry::Pallet<Runtime>>::asset_name(asset_id.into()) {
			Some(name) => {
				log::debug!(target: "evm", "multicurrency: symbol: {:?}", name);

				let encoded = Output::encode_bytes(name.as_slice());

				Ok(succeed(encoded))
			}
			None => Err(PrecompileFailure::Error {
				exit_status: pallet_evm::ExitError::Other("Non-existing asset.".into()),
			}),
		}
	}

	fn symbol(asset_id: AssetId, handle: &mut impl PrecompileHandle) -> PrecompileResult {
		handle.record_cost(RuntimeHelper::<Runtime>::db_read_gas_cost())?;

		// Parse input
		let input = handle.read_input()?;
		input.expect_arguments(0)?;

		match <pallet_asset_registry::Pallet<Runtime>>::asset_symbol(asset_id.into()) {
			Some(symbol) => {
				log::debug!(target: "evm", "multicurrency: name: {:?}", symbol);

				let encoded = Output::encode_bytes(symbol.as_slice());

				Ok(succeed(encoded))
			}
			None => Err(PrecompileFailure::Error {
				exit_status: pallet_evm::ExitError::Other("Non-existing asset.".into()),
			}),
		}
	}

	fn decimals(asset_id: AssetId, handle: &mut impl PrecompileHandle) -> PrecompileResult {
		handle.record_cost(RuntimeHelper::<Runtime>::db_read_gas_cost())?;

		// Parse input
		let input = handle.read_input()?;
		input.expect_arguments(0)?;

		match <pallet_asset_registry::Pallet<Runtime>>::decimals(asset_id.into()) {
			Some(decimals) => {
				log::debug!(target: "evm", "multicurrency: decimals: {:?}", decimals);

				let encoded = Output::encode_uint::<u8>(decimals);

				Ok(succeed(encoded))
			}
			None => Err(PrecompileFailure::Error {
				exit_status: pallet_evm::ExitError::Other("Non-existing asset.".into()),
			}),
		}
	}

	fn total_supply(asset_id: AssetId, handle: &mut impl PrecompileHandle) -> PrecompileResult {
		handle.record_cost(RuntimeHelper::<Runtime>::db_read_gas_cost())?;

		// Parse input
		let input = handle.read_input()?;
		input.expect_arguments(0)?;

		let total_issuance = Currencies::total_issuance(asset_id);

		log::debug!(target: "evm", "multicurrency: totalSupply: {:?}", total_issuance);

		let encoded = Output::encode_uint::<u128>(total_issuance);

		Ok(succeed(encoded))
	}

	fn balance_of(asset_id: AssetId, handle: &mut impl PrecompileHandle) -> PrecompileResult {
		handle.record_cost(RuntimeHelper::<Runtime>::db_read_gas_cost())?;

		// Parse input
		let mut input = handle.read_input()?;
		input.expect_arguments(1)?;

		let owner: H160 = input.read::<Address>()?.into();
		let who: Runtime::AccountId = ExtendedAddressMapping::into_account_id(owner).into(); //TODO: use pallet?

		let free_balance = Currencies::free_balance(asset_id, &who);

		log::debug!(target: "evm", "multicurrency: balanceOf: {:?}", free_balance);

		let encoded = Output::encode_uint::<u128>(free_balance);

		Ok(succeed(encoded))
	}

	fn transfer(asset_id: AssetId, handle: &mut impl PrecompileHandle) -> PrecompileResult {
		handle.record_cost(RuntimeHelper::<Runtime>::db_read_gas_cost())?;

		// Parse input
		let mut input = handle.read_input()?;
		input.expect_arguments(2)?;

		let to: H160 = input.read::<Address>()?.into();
		let amount = input.read::<Balance>()?;

		let origin = ExtendedAddressMapping::into_account_id(handle.context().caller);
		let to = ExtendedAddressMapping::into_account_id(to);

		log::debug!(target: "evm", "multicurrency: transfer from: {:?}, to: {:?}, amount: {:?}", origin, to, amount);

		<pallet_currencies::Pallet<Runtime> as MultiCurrency<Runtime::AccountId>>::transfer(
			asset_id,
			&(<sp_runtime::AccountId32 as Into<Runtime::AccountId>>::into(origin)),
			&(<sp_runtime::AccountId32 as Into<Runtime::AccountId>>::into(to)),
			amount,
		)
		.map_err(|e| PrecompileFailure::Revert {
			exit_status: ExitRevert::Reverted,
			output: Into::<&str>::into(e).as_bytes().to_vec(),
		})?;

		Ok(succeed(EvmDataWriter::new().write(true).build()))
	}

	fn not_supported(_: AssetId, _: &mut impl PrecompileHandle) -> PrecompileResult {
		Err(PrecompileFailure::Error {
			exit_status: pallet_evm::ExitError::Other("not supported".into()),
		})
	}
}
