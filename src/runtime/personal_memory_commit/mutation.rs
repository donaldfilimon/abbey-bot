//! Pure personal fact mutations and exact canonical publication checks.
use super::*;

pub(super) fn validate_action(
    action: &SelfAuthorizedFactAction,
    now: u64,
) -> Result<(), MemoryConsentError> {
    SelfAuthorizedFactAction::new(action.proof.clone(), action.expected)?;
    if action.proof.at == 0 || action.proof.at > now || now - action.proof.at > 300 {
        return Err(MemoryConsentError::InvalidProof);
    }
    Ok(())
}
pub(super) fn subject_epoch(
    stores: &persist::Stores,
    key: &str,
) -> Result<u64, MemoryConsentError> {
    stores
        .personal_memory
        .get(key)
        .map(|s| s.consent_epoch)
        .ok_or(MemoryConsentError::Persistence)
}
pub(super) fn receipt_key(guild: &str, user: &str, text: &str) -> String {
    format!("{guild}\u{1f}{user}\u{1f}{text}")
}
pub(super) fn apply_mutation(
    stores: &mut persist::Stores,
    action: &SelfAuthorizedFactAction,
    mutation: &Mutation,
) -> Result<(), MemoryConsentError> {
    let guild = &action.proof.guild;
    let user = &action.proof.subject;
    let key = subject_key(guild, user);
    let previous = stores
        .personal_memory
        .get(&key)
        .ok_or(MemoryConsentError::Persistence)?
        .revision;
    let mut proof = None;
    match mutation {
        Mutation::Choice(choice) => {
            stores
                .personal_memory
                .get_mut(&key)
                .ok_or(MemoryConsentError::Persistence)?
                .choice = *choice;
        }
        Mutation::Confirm { key: fact, exact } => {
            if !stores.memory.facts(guild, user).contains(exact)
                || fact_key(guild, user, exact) != *fact
            {
                return Err(MemoryConsentError::UnverifiedFact);
            }
            proof = Some((exact.clone(), FactAuthority::MemberConfirmed));
        }
        Mutation::Remember { text, receipt } | Mutation::Correct { text, receipt, .. } => {
            let exact =
                crate::memory::validated_fact(text).map_err(|_| MemoryConsentError::Bounds)?;
            if let Some(receipt) = receipt
                && !valid_digest(receipt)
            {
                return Err(MemoryConsentError::InvalidProof);
            }
            if let Mutation::Correct { old, .. } = mutation {
                if !stores.memory.facts(guild, user).contains(old) {
                    return Err(MemoryConsentError::NotFound);
                }
                stores.memory.forget(guild, user, old);
                stores.memory.drop_supersession(guild, user, old);
                stores
                    .memory_receipts
                    .remove(&receipt_key(guild, user, old));
                stores
                    .personal_memory
                    .get_mut(&key)
                    .ok_or(MemoryConsentError::Persistence)?
                    .proofs
                    .remove(&fact_key(guild, user, old));
            }
            if !stores.memory.facts(guild, user).contains(&exact)
                && !stores.memory.remember(guild, user, &exact, action.proof.at)
            {
                return Err(MemoryConsentError::Bounds);
            }
            if let Some(receipt) = receipt {
                stores
                    .memory_receipts
                    .insert(receipt_key(guild, user, &exact), receipt.clone());
            }
            proof = Some((exact, FactAuthority::SelfAuthored));
        }
        Mutation::Forget { exact } => {
            if !stores.memory.facts(guild, user).contains(exact) {
                return Err(MemoryConsentError::NotFound);
            }
            stores.memory.forget(guild, user, exact);
            stores.memory.drop_supersession(guild, user, exact);
            stores
                .memory_receipts
                .remove(&receipt_key(guild, user, exact));
            stores
                .personal_memory
                .get_mut(&key)
                .ok_or(MemoryConsentError::Persistence)?
                .proofs
                .remove(&fact_key(guild, user, exact));
        }
    }
    if let Some((text, authority)) = proof {
        let fact = fact_key(guild, user, &text);
        let subject = stores
            .personal_memory
            .get_mut(&key)
            .ok_or(MemoryConsentError::Persistence)?;
        subject.proofs.insert(
            fact.clone(),
            FactProof {
                authority,
                member: action.proof.clone(),
                fact_key: fact,
                previous_revision: previous,
            },
        );
    }
    Ok(())
}
pub(super) fn exact_readback(
    dir: &std::path::Path,
    candidate: &persist::Stores,
    guild: &str,
    user: &str,
) -> Result<Option<String>, MemoryConsentError> {
    let expected = serde_json::to_vec(candidate).map_err(|_| MemoryConsentError::Persistence)?;
    let actual = std::fs::read(persist::Stores::state_path(dir))
        .map_err(|_| MemoryConsentError::Persistence)?;
    if expected != actual {
        return Err(MemoryConsentError::Persistence);
    }
    let disk = persist::Stores::load(dir).map_err(|_| MemoryConsentError::Persistence)?;
    if disk.memory.facts(guild, user) != candidate.memory.facts(guild, user) {
        return Err(MemoryConsentError::Persistence);
    }
    Ok(disk.canonical_base.get())
}
pub(super) fn memory_rows(stores: &persist::Stores) -> Vec<(String, String, String, u64)> {
    let mut rows: Vec<_> = stores
        .memory
        .fact_records()
        .into_iter()
        .map(|f| (f.guild, f.user, f.text, f.at))
        .collect();
    rows.sort();
    rows
}
