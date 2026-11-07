"""Reviewed pre-seed policy bytes and the opt-in Dev Container gate contract.

``gate --devcontainer-validation`` keeps the trusted plan and its independent
comparison unchanged. An approved pre-seed policy permits only the supplemental
``devcontainer-validation`` dependency, in addition to every prior job. It must
succeed and report the exact tested ``outputs.candidate_sha`` from current needs.
Seeded inventories use the normal gate, with a candidate receipt required for
any successful image job; an explicitly unselected skip needs no receipt.

Entries recognize only the independently reviewed actual-main policy bytes
recorded in tests/fixtures/devcontainer_seed/. New bytes need a fresh
main-policy read and complete prior-gate review. Future draft hashes and
candidate-supplied values are never approval data. Missing policies have no bypass.
This interface activates no consumer or updater caller;
native image qualification, budgets and App evidence remain separate work.
"""

from __future__ import annotations

import hashlib
from collections.abc import Mapping
from types import MappingProxyType

from boxferry_ci.contracts import ContractError
from boxferry_ci.policy import Policy, load_policy

DEVCONTAINER_JOB = "devcontainer-validation"
APPROVED_SEED_POLICIES: Mapping[str, frozenset[str]] = MappingProxyType(
    {
        "Strukturpiloten/boxferry": frozenset(
            {"b83045f7d6b17c8874f414536e9d8aa87ddb9e957df236c3f70501bab936cb52"}
        ),
        "Strukturpiloten/compose-lens": frozenset(
            {
                "514efbfda2d38bbf5e608d0785e8f0d8be97af1083733b966146a7f0273ab6a4",
                "7957938d108af2e6799befc95b19d12d32d1bc37f97d0bc1118e6d8889d36c73",
            }
        ),
        "Strukturpiloten/podman-lens": frozenset(
            {
                "f21647319d0829cca3e36ac2fd43e21595415ab80bdbc5faedce92954d7f96cd",
                "0e80da07f01f66e5963c423769d0b9d6ecf92fb131437a76b2828c84c979462e",
            }
        ),
        "Strukturpiloten/quadlet-lens": frozenset(
            {
                "29aff089e579582fb8ed5eb5e681b6c3f170c61a17bc188b82808b12f7e01ac3",
                "d4a88fb53131ebbea02578d532d14ed8e1c5581efc162a30a9f61c7cc522ee90",
            }
        ),
    }
)


def require_devcontainer_policy(policy: Policy, *, seed: bool) -> None:
    """Admit only four owners and, during seed, exact approved prior policy bytes."""
    if policy.repository not in APPROVED_SEED_POLICIES:
        raise ContractError("Dev Container gate is restricted to the four reviewed owners")
    if not seed:
        return
    digest = hashlib.sha256(policy.path.read_bytes()).hexdigest()
    if digest not in APPROVED_SEED_POLICIES[policy.repository]:
        raise ContractError(
            "unapproved pre-seed Dev Container policy; review exact trusted main policy bytes "
            "before seed activation"
        )
    if load_policy(policy.path) != policy:
        raise ContractError("pre-seed policy object differs from its approved policy bytes")
