use crate::*;
use frame_support::{
    dispatch::DispatchResult, ensure, sp_runtime::DispatchError, storage::with_storage_layer,
    traits::ConstU32, BoundedBTreeMap, BoundedBTreeSet, BoundedVec, DebugNoBound,
};
use frame_system::ensure_signed;
use pallet_subspace::{
    Event as SubspaceEvent, GlobalParams, Pallet as PalletSubspace, SubnetParams,
};
use parity_scale_codec::{Decode, Encode, MaxEncodedLen};
use scale_info::TypeInfo;
use sp_std::{collections::btree_set::BTreeSet, vec::Vec};
use substrate_fixed::types::I92F36;

pub type ProposalId = u64;

#[derive(DebugNoBound, TypeInfo, Decode, Encode, MaxEncodedLen)]
#[scale_info(skip_type_params(T))]
pub struct Proposal<T: Config> {
    pub id: ProposalId,
    pub proposer: T::AccountId,
    pub expiration_block: u64,
    pub data: ProposalData<T>,
    pub status: ProposalStatus<T>,
    pub metadata: BoundedVec<u8, ConstU32<256>>,
    pub proposal_cost: u64,
    pub creation_block: u64,
}

impl<T: Config> Proposal<T> {
    /// Whether the proposal is still active.
    #[must_use]
    pub fn is_active(&self) -> bool {
        matches!(self.status, ProposalStatus::Open { .. })
    }

    /// Returns the subnet ID that this proposal impact.s
    #[must_use]
    pub fn subnet_id(&self) -> Option<u16> {
        match &self.data {
            ProposalData::SubnetParams { subnet_id, .. }
            | ProposalData::SubnetCustom { subnet_id, .. } => Some(*subnet_id),
            _ => None,
        }
    }

    /// Marks a proposal as accepted and overrides the storage value.
    pub fn accept(mut self, block: u64, stake_for: u64, stake_against: u64) -> DispatchResult {
        ensure!(self.is_active(), Error::<T>::ProposalIsFinished);

        self.status = ProposalStatus::Accepted {
            block,
            stake_for,
            stake_against,
        };

        Proposals::<T>::insert(self.id, &self);
        Pallet::<T>::deposit_event(Event::ProposalAccepted(self.id));

        self.execute_proposal()?;

        Ok(())
    }

    /// Marks a proposal as accepted by the senate
    pub fn senate_accept(mut self, block: u64) -> DispatchResult {
        ensure!(self.is_active(), Error::<T>::ProposalIsFinished);

        self.status = ProposalStatus::AcceptedBySenate { block };

        Proposals::<T>::insert(self.id, &self);
        Pallet::<T>::deposit_event(Event::ProposalAcceptedBySenate(self.id));

        self.execute_proposal()?;

        Ok(())
    }

    fn execute_proposal(self) -> DispatchResult {
        PalletSubspace::<T>::add_balance_to_account(
            &self.proposer,
            PalletSubspace::<T>::u64_to_balance(self.proposal_cost).unwrap(),
        );

        match self.data {
            ProposalData::GlobalCustom | ProposalData::SubnetCustom { .. } => {
                // No specific action needed for custom proposals
                // The owners will handle the off-chain logic
            }
            ProposalData::GlobalParams(params) => {
                PalletSubspace::<T>::set_global_params(params.clone())?;
                PalletSubspace::<T>::deposit_event(SubspaceEvent::GlobalParamsUpdated(params));
            }
            ProposalData::SubnetParams { subnet_id, params } => {
                let changeset = SubnetChangeset::<T>::update(subnet_id, params)?;
                changeset.apply(subnet_id)?;
                PalletSubspace::<T>::deposit_event(SubspaceEvent::SubnetParamsUpdated(subnet_id));
            }
            ProposalData::TransferDaoTreasury { account, amount } => {
                PalletSubspace::<T>::transfer_balance_to_account(
                    &DaoTreasuryAddress::<T>::get(),
                    &account,
                    amount,
                )?;
            }
        }

        Ok(())
    }

    /// Marks a proposal as refused and overrides the storage value.
    pub fn refuse(mut self, block: u64, stake_for: u64, stake_against: u64) -> DispatchResult {
        ensure!(self.is_active(), Error::<T>::ProposalIsFinished);

        self.status = ProposalStatus::Refused {
            block,
            stake_for,
            stake_against,
        };

        Proposals::<T>::insert(self.id, &self);
        Pallet::<T>::deposit_event(Event::ProposalRefused(self.id));

        Ok(())
    }

    /// Marks a proposal as accepted by the senate
    pub fn senate_refuse(mut self, block: u64) -> DispatchResult {
        ensure!(self.is_active(), Error::<T>::ProposalIsFinished);

        self.status = ProposalStatus::RefusedBySenate { block };

        Proposals::<T>::insert(self.id, &self);
        Pallet::<T>::deposit_event(Event::ProposalRefusedBySenate(self.id));

        Ok(())
    }

    /// Marks a proposal as expired and overrides the storage value.
    pub fn expire(mut self, block_number: u64) -> DispatchResult {
        ensure!(self.is_active(), Error::<T>::ProposalIsFinished);
        ensure!(
            block_number >= self.expiration_block,
            Error::<T>::InvalidProposalFinalizationParameters
        );

        self.status = ProposalStatus::Expired;

        Proposals::<T>::insert(self.id, &self);
        Pallet::<T>::deposit_event(Event::ProposalExpired(self.id));

        Ok(())
    }
}

#[derive(Clone, DebugNoBound, TypeInfo, Decode, Encode, MaxEncodedLen, PartialEq, Eq)]
#[scale_info(skip_type_params(T))]
pub enum ProposalStatus<T: Config> {
    Open {
        votes_for: BoundedBTreeSet<T::AccountId, ConstU32<{ u32::MAX }>>,
        votes_against: BoundedBTreeSet<T::AccountId, ConstU32<{ u32::MAX }>>,
        stake_for: u64,
        stake_against: u64,
    },
    Accepted {
        block: u64,
        stake_for: u64,
        stake_against: u64,
    },
    AcceptedBySenate {
        block: u64,
    },
    Refused {
        block: u64,
        stake_for: u64,
        stake_against: u64,
    },
    RefusedBySenate {
        block: u64,
    },
    Expired,
}

#[derive(DebugNoBound, TypeInfo, Decode, Encode, MaxEncodedLen, PartialEq, Eq)]
#[scale_info(skip_type_params(T))]
pub enum ProposalData<T: Config> {
    GlobalCustom,
    GlobalParams(pallet_subspace::GlobalParams<T>),
    SubnetCustom {
        subnet_id: SubnetId,
    },
    SubnetParams {
        subnet_id: SubnetId,
        params: pallet_subspace::SubnetParams<T>,
    },
    TransferDaoTreasury {
        account: T::AccountId,
        amount: u64,
    },
}

#[derive(DebugNoBound, TypeInfo, Decode, Encode, MaxEncodedLen, PartialEq, Eq)]
#[scale_info(skip_type_params(T))]
pub struct UnrewardedProposal<T: Config> {
    pub subnet_id: Option<SubnetId>,
    pub block: u64,
    pub votes_for: BoundedBTreeMap<T::AccountId, u64, ConstU32<{ u32::MAX }>>,
    pub votes_against: BoundedBTreeMap<T::AccountId, u64, ConstU32<{ u32::MAX }>>,
}

impl<T: Config> Pallet<T> {
    fn get_next_proposal_id() -> u64 {
        match Proposals::<T>::iter_keys().max() {
            Some(id) => id.saturating_add(1),
            None => 0,
        }
    }

    pub fn add_proposal(
        key: T::AccountId,
        metadata: BoundedVec<u8, ConstU32<256>>,
        data: ProposalData<T>,
    ) -> DispatchResult {
        let GovernanceConfiguration {
            proposal_cost,
            proposal_expiration,
            ..
        } = GlobalGovernanceConfig::<T>::get();

        ensure!(
            pallet_subspace::Pallet::<T>::has_enough_balance(&key, proposal_cost),
            Error::<T>::NotEnoughBalanceToPropose
        );

        let Some(removed_balance_as_currency) = PalletSubspace::<T>::u64_to_balance(proposal_cost)
        else {
            return Err(Error::<T>::InvalidCurrencyConversionValue.into());
        };

        let proposal_id = Self::get_next_proposal_id();
        let current_block = PalletSubspace::<T>::get_current_block_number();
        let expiration_block = current_block.saturating_add(proposal_expiration as u64);

        // TODO: extract rounding function
        let expiration_block = if expiration_block % 100 == 0 {
            expiration_block
        } else {
            expiration_block
                .saturating_add(100)
                .saturating_sub(expiration_block.checked_rem(100).unwrap_or_default())
        };

        let proposal = Proposal {
            id: proposal_id,
            proposer: key.clone(),
            expiration_block,
            data,
            status: ProposalStatus::Open {
                votes_for: BoundedBTreeSet::new(),
                votes_against: BoundedBTreeSet::new(),
                stake_for: 0,
                stake_against: 0,
            },
            proposal_cost,
            creation_block: current_block,
            metadata,
        };

        // Burn the proposal cost from the proposer's balance
        PalletSubspace::<T>::remove_balance_from_account(&key, removed_balance_as_currency)?;

        Proposals::<T>::insert(proposal_id, proposal);

        Self::deposit_event(Event::<T>::ProposalCreated(proposal_id));
        Ok(())
    }

    pub fn do_add_global_custom_proposal(
        origin: T::RuntimeOrigin,
        data: Vec<u8>,
    ) -> DispatchResult {
        let key = ensure_signed(origin)?;
        ensure!(!data.is_empty(), Error::<T>::ProposalDataTooSmall);
        ensure!(data.len() <= 256, Error::<T>::ProposalDataTooLarge);
        sp_std::str::from_utf8(&data).map_err(|_| Error::<T>::InvalidProposalData)?;

        let proposal_data = ProposalData::GlobalCustom;
        Self::add_proposal(key, BoundedVec::truncate_from(data), proposal_data)
    }

    pub fn do_add_subnet_custom_proposal(
        origin: T::RuntimeOrigin,
        subnet_id: u16,
        data: Vec<u8>,
    ) -> DispatchResult {
        let key = ensure_signed(origin)?;
        ensure!(!data.is_empty(), Error::<T>::ProposalDataTooSmall);
        ensure!(data.len() <= 256, Error::<T>::ProposalDataTooLarge);
        sp_std::str::from_utf8(&data).map_err(|_| Error::<T>::InvalidProposalData)?;

        let proposal_data = ProposalData::SubnetCustom { subnet_id };
        Self::add_proposal(key, BoundedVec::truncate_from(data), proposal_data)
    }

    pub fn do_add_transfer_dao_treasury_proposal(
        origin: T::RuntimeOrigin,
        data: Vec<u8>,
        value: u64,
        dest: T::AccountId,
    ) -> DispatchResult {
        let key = ensure_signed(origin)?;
        ensure!(!data.is_empty(), Error::<T>::ProposalDataTooSmall);
        ensure!(data.len() <= 256, Error::<T>::ProposalDataTooLarge);
        ensure!(
            pallet_subspace::Pallet::<T>::has_enough_balance(
                &DaoTreasuryAddress::<T>::get(),
                value
            ),
            Error::<T>::InsufficientDaoTreasuryFunds
        );
        sp_std::str::from_utf8(&data).map_err(|_| Error::<T>::InvalidProposalData)?;

        let proposal_data = ProposalData::TransferDaoTreasury {
            amount: value,
            account: dest,
        };
        Self::add_proposal(key, BoundedVec::truncate_from(data), proposal_data)
    }

    pub fn do_add_global_params_proposal(
        origin: T::RuntimeOrigin,
        data: Vec<u8>,
        mut params: GlobalParams<T>,
    ) -> DispatchResult {
        let key = ensure_signed(origin)?;

        ensure!(!data.is_empty(), Error::<T>::ProposalDataTooSmall);
        ensure!(data.len() <= 256, Error::<T>::ProposalDataTooLarge);

        params.governance_config = Self::validate(params.governance_config)?;
        pallet_subspace::Pallet::check_global_params(&params)?;

        let proposal_data = ProposalData::GlobalParams(params);
        Self::add_proposal(key, BoundedVec::truncate_from(data), proposal_data)
    }

    pub fn do_add_subnet_params_proposal(
        origin: T::RuntimeOrigin,
        subnet_id: u16,
        data: Vec<u8>,
        mut params: SubnetParams<T>,
    ) -> DispatchResult {
        let key = ensure_signed(origin)?;

        ensure!(
            matches!(
                SubnetGovernanceConfig::<T>::get(subnet_id).vote_mode,
                VoteMode::Vote
            ),
            Error::<T>::NotVoteMode
        );

        ensure!(!data.is_empty(), Error::<T>::ProposalDataTooSmall);
        ensure!(data.len() <= 256, Error::<T>::ProposalDataTooLarge);

        params.governance_config = Self::validate(params.governance_config)?;
        SubnetChangeset::<T>::update(subnet_id, params.clone())?;

        let proposal_data = ProposalData::SubnetParams { subnet_id, params };
        Self::add_proposal(key, BoundedVec::truncate_from(data), proposal_data)
    }
}

pub fn tick_proposals<T: Config>(block_number: u64) {
    let not_delegating = NotDelegatingVotingPower::<T>::get().into_inner();

    let proposals = Proposals::<T>::iter().filter(|(_, p)| p.is_active());

    if block_number % 100 != 0 {
        return;
    }

    for (id, proposal) in proposals {
        let res = with_storage_layer(|| tick_proposal(&not_delegating, block_number, proposal));
        if let Err(err) = res {
            log::error!("failed to tick proposal {id}: {err:?}, skipping...");
        }
    }
}

pub fn check_senate_approval<T: Config>(proposal: Proposal<T>) -> DispatchResult {
    let block_number: u64 = <frame_system::Pallet<T>>::block_number()
        .try_into()
        .ok()
        .expect("Blocks will not exceed u64 maximum.");

    let ProposalStatus::Open {
        votes_for,
        votes_against,
        ..
    } = &proposal.status
    else {
        return Err(Error::<T>::ProposalIsFinished.into());
    };

    let senate_for: u32 = votes_for
        .iter()
        .filter(|id| SenateMembers::<T>::contains_key(id))
        .cloned()
        .collect::<Vec<_>>()
        .len() as u32;
    let senate_against: u32 = votes_against
        .iter()
        .filter(|id| SenateMembers::<T>::contains_key(id))
        .cloned()
        .collect::<Vec<_>>()
        .len() as u32;
    let total_senate_members = SenateMembers::<T>::iter().count() as u32;

    // Check if the senate_for count meets the 4/7 ratio of total senate members
    let mut senate_threshold = total_senate_members.saturating_mul(4).saturating_div(7);
    // If the threshold is exactly 50% of the total, increase by 1
    if senate_threshold.saturating_mul(2) == total_senate_members {
        senate_threshold = senate_threshold.saturating_add(1);
    }
    // Senate agreement overrides DAO (for now)
    if senate_for >= senate_threshold {
        return proposal.senate_accept(block_number);
    } else if senate_against >= senate_threshold {
        return proposal.senate_refuse(block_number);
    }
    Ok(())
}

fn tick_proposal<T: Config>(
    not_delegating: &BTreeSet<T::AccountId>,
    block_number: u64,
    mut proposal: Proposal<T>,
) -> DispatchResult {
    let ProposalStatus::Open {
        votes_for,
        votes_against,
        ..
    } = &proposal.status
    else {
        return Err(Error::<T>::ProposalIsFinished.into());
    };

    let votes_for: Vec<(T::AccountId, u64)> = votes_for
        .iter()
        .cloned()
        .map(|id| {
            if SenateMembers::<T>::contains_key(id.clone()) {
                (id, 0u64)
            } else {
                let stake = calc_stake::<T>(not_delegating, &id);
                (id, stake)
            }
        })
        .collect();
    let votes_against: Vec<(T::AccountId, u64)> = votes_against
        .iter()
        .cloned()
        .map(|id| {
            if SenateMembers::<T>::contains_key(id.clone()) {
                (id, 0u64)
            } else {
                let stake = calc_stake::<T>(not_delegating, &id);
                (id, stake)
            }
        })
        .collect();

    let stake_for_sum: u64 = votes_for.iter().map(|(_, stake)| *stake).sum();
    let stake_against_sum: u64 = votes_against.iter().map(|(_, stake)| *stake).sum();

    if block_number < proposal.expiration_block {
        if let ProposalStatus::Open {
            stake_for,
            stake_against,
            ..
        } = &mut proposal.status
        {
            *stake_for = stake_for_sum;
            *stake_against = stake_against_sum;
        }
        Proposals::<T>::set(proposal.id, Some(proposal));
        return Ok(());
    }

    let mut reward_votes_for = BoundedBTreeMap::new();
    for (key, value) in votes_for {
        reward_votes_for.try_insert(key, value).expect("this wont exceed u32::MAX");
    }

    let mut reward_votes_against: BoundedBTreeMap<T::AccountId, u64, ConstU32<{ u32::MAX }>> =
        BoundedBTreeMap::new();
    for (key, value) in votes_against {
        reward_votes_against
            .try_insert(key, value)
            .expect("this probably wont exceed u32::MAX");
    }

    UnrewardedProposals::<T>::insert(
        proposal.id,
        UnrewardedProposal::<T> {
            subnet_id: proposal.subnet_id(),
            block: block_number,
            votes_for: reward_votes_for,
            votes_against: reward_votes_against,
        },
    );

    if stake_against_sum > stake_for_sum {
        proposal.refuse(block_number, stake_for_sum, stake_against_sum)
    } else {
        proposal.accept(block_number, stake_for_sum, stake_against_sum)
    }
}

pub fn tick_proposal_rewards<T: Config>(block_number: u64) {
    let mut to_tick: Vec<_> = pallet_subspace::N::<T>::iter_keys()
        .map(|subnet_id| (Some(subnet_id), SubnetGovernanceConfig::<T>::get(subnet_id)))
        .collect();
    to_tick.push((None, GlobalGovernanceConfig::<T>::get()));

    to_tick.into_iter().for_each(|(subnet_id, governance_config)| {
        execute_proposal_rewards::<T>(block_number, subnet_id, governance_config);
    });
}

#[inline]
fn calc_stake<T: Config>(not_delegating: &BTreeSet<T::AccountId>, voter: &T::AccountId) -> u64 {
    let own_stake = if !not_delegating.contains(voter) {
        0
    } else {
        pallet_subspace::Pallet::<T>::get_owned_stake(voter)
    };

    let calculate_delegated = || -> u64 {
        PalletSubspace::<T>::get_stake_from_vector(voter)
            .into_iter()
            .filter(|(staker, _)| !not_delegating.contains(staker))
            .map(|(_, stake)| stake)
            .sum()
    };

    let delegated_stake = calculate_delegated();

    own_stake.saturating_add(delegated_stake)
}

pub fn execute_proposal_rewards<T: Config>(
    block_number: u64,
    subnet_id: Option<u16>,
    governance_config: GovernanceConfiguration,
) {
    let reached_interval = block_number
        .checked_rem(governance_config.proposal_reward_interval)
        .is_some_and(|r| r == 0);
    if !reached_interval {
        return;
    }

    let mut n: u16 = 0;
    let mut account_stakes: BoundedBTreeMap<T::AccountId, u64, ConstU32<{ u32::MAX }>> =
        BoundedBTreeMap::new();
    let mut total_allocation: I92F36 = I92F36::from_num(0);
    for (proposal_id, unrewarded_proposal) in UnrewardedProposals::<T>::iter() {
        if subnet_id != unrewarded_proposal.subnet_id {
            continue;
        }

        if unrewarded_proposal.block
            < block_number.saturating_sub(governance_config.proposal_reward_interval)
        {
            continue;
        }

        for (acc_id, stake) in unrewarded_proposal
            .votes_for
            .into_iter()
            .chain(unrewarded_proposal.votes_against.into_iter())
        {
            let curr_stake = *account_stakes.get(&acc_id).unwrap_or(&0u64);
            account_stakes
                .try_insert(acc_id, curr_stake.saturating_add(stake))
                .expect("infallible");
        }

        match get_reward_allocation::<T>(&governance_config, n) {
            Ok(allocation) => {
                total_allocation = total_allocation.saturating_add(allocation);
            }
            Err(err) => {
                log::error!("could not get reward allocation for proposal {proposal_id}: {err:?}");
                continue;
            }
        }

        UnrewardedProposals::<T>::remove(proposal_id);
        n = n.saturating_add(1);
    }

    distribute_proposal_rewards::<T>(
        account_stakes,
        total_allocation,
        governance_config.max_proposal_reward_treasury_allocation,
    );
}

pub fn get_reward_allocation<T: crate::Config>(
    governance_config: &GovernanceConfiguration,
    n: u16,
) -> Result<I92F36, DispatchError> {
    let treasury_address = DaoTreasuryAddress::<T>::get();
    let treasury_balance = pallet_subspace::Pallet::<T>::get_balance(&treasury_address);
    let treasury_balance = I92F36::from_num(pallet_subspace::Pallet::<T>::balance_to_u64(
        treasury_balance,
    ));

    let allocation_percentage =
        I92F36::from_num(governance_config.proposal_reward_treasury_allocation.deconstruct());
    let max_allocation =
        I92F36::from_num(governance_config.max_proposal_reward_treasury_allocation);

    let mut allocation = treasury_balance
        .checked_mul(allocation_percentage)
        .unwrap_or_default()
        .min(max_allocation);
    if n > 0 {
        let mut base = I92F36::from_num(1.5);
        let mut result = I92F36::from_num(1);
        let mut remaining = n;

        while remaining > 0 {
            if remaining % 2 == 1 {
                result = result.checked_mul(base).unwrap_or(result);
            }
            base = base.checked_mul(base).unwrap_or_default();
            remaining /= 2;
        }

        allocation = allocation.checked_div(result).unwrap_or(allocation);
    }
    Ok(allocation)
}

fn distribute_proposal_rewards<T: crate::Config>(
    account_stakes: BoundedBTreeMap<T::AccountId, u64, ConstU32<{ u32::MAX }>>,
    total_allocation: I92F36,
    max_proposal_reward_treasury_allocation: u64,
) {
    // This is just a sanity check, making sure we can never allocate more than the max
    if total_allocation > I92F36::from_num(max_proposal_reward_treasury_allocation) {
        log::error!("total allocation exceeds max proposal reward treasury allocation");
        return;
    }

    use frame_support::sp_runtime::traits::IntegerSquareRoot;

    let dao_treasury_address = DaoTreasuryAddress::<T>::get();
    let account_sqrt_stakes: Vec<_> = account_stakes
        .into_iter()
        .map(|(acc_id, stake)| (acc_id, stake.integer_sqrt()))
        .collect();

    let total_stake: u64 = account_sqrt_stakes.iter().map(|(_, stake)| *stake).sum();
    let total_stake = I92F36::from_num(total_stake);

    for (acc_id, stake) in account_sqrt_stakes.into_iter() {
        let percentage = I92F36::from_num(stake).checked_div(total_stake).unwrap_or_default();

        let reward: u64 = total_allocation.checked_mul(percentage).unwrap_or_default().to_num();

        // Transfer the proposal reward to the accounts from treasury
        let _ = PalletSubspace::<T>::transfer_balance_to_account(
            &dao_treasury_address,
            &acc_id,
            reward,
        );
    }
}
