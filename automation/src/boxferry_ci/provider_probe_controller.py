#!/usr/bin/env python3
"""Manual non-authorizing probe control, derived from the preserved authored v5 draft.

No polling, retries, existing-run attachment, force cancellation or production effects.
Exact source and package identity come from independently reviewed fresh-series custody.
"""

from __future__ import annotations

import argparse
import base64
import datetime as dt
import functools
import hashlib
import http.client
import json
import os
import re
import urllib.error
import urllib.request
from collections.abc import Callable
from pathlib import Path
from types import ModuleType, SimpleNamespace
from typing import Any, cast

CONTROLLER_SCHEMA = "boxferry.provider-probe.controller.v1"
REPOSITORY = "Strukturpiloten/boxferry"
PREFIX = "/repos/" + REPOSITORY
API = "https://api.github.com"
WORKFLOW = ".github/workflows/recovery-cancellation-provider-probe.yml"


# Fixed source-only diagnostic vocabulary. Never render exception text or response fields.
REFUSAL_CODES = {
    "A bound identities changed": "O001",
    "A must be first": "C001",
    "API endpoint outside closed read catalog": "O002",
    "B pending before fresh A proof missing": "C002",
    "B pending/terminal sequence unobserved": "O003",
    "D cancellation not accepted": "C003",
    "D dependent entry contradiction or unsupported dependent disposition": "C004",
    "D positive failed preparation missing": "C005",
    "E accepted cancel followed by natural or ambiguous effect completion": "V4O001",
    "E active or naturally completed effect log forbidden": "V4O002",
    "E actual root cancellation unavailable": "V4O003",
    "E archive not enumerated": "V4O004",
    "E attributed preparation archive must precede final effect hold": "V4O005",
    "E authenticated job/check or inventory contradiction": "V4O006",
    "E authenticated observation custody unavailable": "V4O007",
    "E authenticated observation unavailable": "V4O008",
    "E authenticated phased uploader log unavailable": "V4O009",
    "E bare digest cannot replace provenance": "V4O010",
    "E caller wrapper retention contradictory or unsupported": "V4O011",
    "E cancellation boundary differs from fresh frozen cut": "V4O012",
    "E complete inventory custody unavailable": "V4O013",
    "E complete inventory unavailable": "V4O014",
    "E copied mandatory pre-cancel proof incomplete or changed": "V4C001",
    "E cut must be frozen and copied before cancellation invocation": "V4C002",
    "E draft source identity mismatch": "V4O015",
    "E duplicate artifact role": "V4O016",
    "E duplicate or invalid step numbers": "V4O017",
    "E duplicate or parallel role identity": "V4O018",
    "E durable copy identity cut or phase mismatch": "V4C003",
    "E durable custody directory symlink refused": "V4C004",
    "E effect inherited preparation linkage mismatch": "V4O019",
    "E entry metadata ID or content changed": "V4O020",
    "E entry metadata not observed before cancellation": "V4O021",
    "E exact check identity unavailable": "V4O022",
    "E exact effect hold phase unavailable": "V4O023",
    "E exact helper-bound pre-cancel cut required": "V4C005",
    "E exact operator uploader span binding unavailable": "V4O024",
    "E exact source manifest unavailable": "V4O025",
    "E exact source step missing or duplicated": "V4O026",
    "E explicit uploader evidence phase required": "V4O027",
    "E exposed role lacks post-inventory authentication": "V4O028",
    "E final effect provenance has wrong phase time": "V4O029",
    "E final evidence lacks exact durable pre-cancel cut readback": "V4O030",
    "E final qualification lacks spent accepted planned cancellation": "V4O031",
    "E final uploader artifact differs from pre-cancel observed ID": "V4O032",
    "E first and spaced stable terminal inventories required": "V4O033",
    "E first-terminal role authentication missing before stable inventory": "V4O034",
    "E frozen cut custody missing at qualification": "V4O035",
    "E frozen cut is not a prefix of pre-cancel boundary": "V4C006",
    "E frozen pre-cancel cut unavailable for qualification": "V4O036",
    "E frozen pre-cancel proof changed": "V4O037",
    "E indexed evidence binding unavailable": "V4O038",
    "E indexed evidence bytes changed": "V4O039",
    "E indexed evidence shape changed": "V4O040",
    "E inventory hold preparation phase order invalid": "V4O041",
    "E new replaced or unmapped observed role": "V4O042",
    "E observed effect wrapper contradicts active leaf": "V5O001",
    "E observed preparation wrapper contradicts success": "V4O043",
    "E operator exact uploader span lacks source digest or artifact ID": "V4O044",
    "E phase-specific verified archive unavailable": "V4O045",
    "E post-cancel archive cannot enter pre-cancel proof": "V4O046",
    "E post-cancel uploader lacks accepted same-effect cancellation": "V4O047",
    "E pre-cancel active complete inventory required": "V4O048",
    "E pre-cancel cut custody missing": "V4O049",
    "E pre-cancel cut integrity mismatch": "V4O050",
    "E pre-cancel full inventory required": "C006",
    "E pre-cancel guard cannot run after reservation": "V4O051",
    "E preparation archive or provenance changed since cut": "V4O052",
    "E preparation attribution cannot be filled after cut or cancellation": "V4O053",
    "E preparation success changed after cancellation": "V4O054",
    "E qualification arguments differ from frozen cut": "V4O055",
    "E required upload or validation did not succeed": "V4O056",
    "E reviewed storage attestation identity mismatch": "V4C007",
    "E reviewed storage attestation unavailable": "V4C008",
    "E run identities differ": "O004",
    "E same effect cancelled job/check unobserved": "V4O057",
    "E same preparation post-stable authentication missing": "V4O058",
    "E source catalog binding mismatch": "V4O059",
    "E source role mapping unavailable": "V4O060",
    "E source step identity changed": "V4O061",
    "E source steps unavailable": "V4O062",
    "E stable artifact metadata changed": "V4O063",
    "E structured uploader provenance unavailable": "V4O064",
    "E terminal cancellation unobserved": "O005",
    "E terminal effect wrapper contradicts cancelled leaf": "V5O002",
    "E terminal evidence predates accepted cancellation phase close": "V4O065",
    "E terminal new replaced parallel identity": "V4O066",
    "E terminal preparation wrapper contradicts retention": "V4O067",
    "E terminal stable inventory required before effect log": "V4O068",
    "E unique preparation and entry metadata required": "V4O069",
    "E unique required expanded leaves unavailable": "V4O070",
    "E uploader artifact digest or source step mismatch": "V4O071",
    "E uploader artifact was not enumerated": "V4O072",
    "E uploader exact line span unavailable": "V4O073",
    "E uploader job/check authentication unavailable": "V4O074",
    "E uploader log provenance binding changed": "V4O075",
    "E uploader observation changed after log collection": "V4O076",
    "E uploader provenance already bound; never replace phase": "V4O077",
    "E uploader provenance custody changed": "V4O078",
    "E uploader provenance time order invalid": "V4O079",
    "E uploader raw log binding changed": "V4O080",
    "E uploader role phase mismatch": "V4O081",
    "E uploader source step identity mismatch": "V4O082",
    "E uploader span invalid": "V4O083",
    "E v5 controller identity unavailable": "V4O084",
    "E validation upload hold order invalid": "V4O085",
    "E verified archive custody mismatch": "V4O086",
    "GET route refused": "C007",
    "actor identity drift": "O006",
    "aggregate custody byte budget exhausted": "O007",
    "aggregate log byte budget exhausted": "O008",
    "another observer invocation holds custody lock": "O009",
    "another probe run is active": "C008",
    "approved workflow bytes mismatch": "C009",
    "approved workflow mode/blob mismatch": "C010",
    "archive exceeds bound": "O010",
    "archive member is not one regular unencrypted file": "O011",
    "archive size/server/uploader digest mismatch": "O012",
    "artifact expiry binding invalid": "O013",
    "artifact expiry metadata unavailable": "O014",
    "artifact metadata changed": "O015",
    "artifact metadata must be an object": "O016",
    "artifact outside exact probe namespace": "O017",
    "artifact run/source attribution unavailable": "O018",
    "artifact server digest unavailable": "O019",
    "artifact unavailable or oversized": "O020",
    "artifact was not enumerated for exact run": "O021",
    "authenticated successful control leaves after stable inventory required": "C011",
    "blob unavailable": "C012",
    "both control witness roles required": "C013",
    "both verified E witnesses required before cancellation": "O022",
    "both verified successful-control archives required": "C014",
    "bounded GET request or response refused": "O023",
    "bounded GET transport failed": "O024",
    "bounded ambiguous dispatch attempt window unavailable": "C015",
    "bounded dispatch attempt end unavailable": "C016",
    "burst cases do not match A/B": "O025",
    "cancellation already spent; never retry": "C017",
    "cannot attach controller to existing runs or reset preflight": "C018",
    "cannot control an existing or unrelated run": "C019",
    "cannot dispatch/adopt an already-bound case": "C020",
    "case already bound; never replace run": "C021",
    "case already bound; no replacement runs": "O026",
    "case discovery read allowance exhausted": "C022",
    "check job linkage unavailable": "O027",
    "check provider/source drift": "O028",
    "clipped or invalid inventory": "O029",
    "complete stable control artifact inventory required": "C023",
    "complete stable role identity unavailable": "O030",
    "complete successful-control inventories unavailable": "C024",
    "control archive custody shape mismatch": "C025",
    "control witness differs from its verified bytes": "C026",
    "controller GET object unavailable": "C027",
    "controller GET unavailable": "C028",
    "controller bytes changed during series": "C029",
    "controller main CI identity mismatch": "C030",
    "controller redirects refused": "C031",
    "controller response exceeds bound": "C032",
    "controller response privacy refusal; spent effect remains consumed": "C033",
    "controller source binding mismatch": "C034",
    "controller transport outcome unavailable; never retry effect": "C035",
    "credential origin refused": "O031",
    "custody already exists; series budget cannot be reset": "O032",
    "default main moved or source identity unavailable": "C036",
    "dependent entry upload success unobserved": "O033",
    "dispatch attempt UTC ordering invalid": "C037",
    "download path outside closed catalog": "O034",
    "download redirect bound exhausted": "O035",
    "download unavailable": "O036",
    "duplicate JSON field": "O037",
    "duplicate control witness role": "C038",
    "duplicate discovery identity": "C039",
    "duplicate inventory identity": "O038",
    "duplicate probe artifact names": "O039",
    "duplicate role records unsupported": "O040",
    "durable E proof required before cancellation": "C040",
    "durable copy identity mismatch": "C041",
    "durable pre-cancel proof set unavailable": "C042",
    "durable proof file shape mismatch": "C043",
    "durable proof index missing or changed": "C044",
    "durable readback mismatch": "C045",
    "durable state exceeds bound": "C046",
    "durable state file exceeds bound": "C047",
    "effect already spent; ambiguity never permits retry": "C048",
    "effect response unsupported; reservation remains spent": "C049",
    "effect route refused": "C050",
    "event/ref/source drift": "O041",
    "evidence file collision": "O042",
    "exact source main CI has not succeeded": "C051",
    "expanded role identity unobserved": "O043",
    "explicit cleanup reason required": "C052",
    "explicit job binding drift": "O044",
    "five-second reread cadence not reached": "O045",
    "fixed hold step identity changed": "O046",
    "fresh A hold observation needed": "C053",
    "fresh A post-B terminal observation unavailable": "O047",
    "fresh positive hold evidence required; no remaining-time inference": "C054",
    "fresh preparation job/check read must follow terminal inventory": "O048",
    "host reboot invalidates series clock": "O049",
    "incomplete pagination": "O050",
    "inherited preparation ID unavailable": "O051",
    "inherited preparation artifact not enumerated": "O052",
    "invalid SHA256": "O053",
    "invalid actor": "O054",
    "invalid bounded JSON": "O055",
    "invalid bounded witness archive": "O056",
    "invalid case": "O057",
    "invalid control archive custody path": "C055",
    "invalid evidence label": "O058",
    "invalid local credential shape": "O059",
    "invalid phase label": "O060",
    "invalid positive provider identity": "O061",
    "invalid preflight start timestamp": "O062",
    "invalid previously spent preflight request count": "O063",
    "invalid redirect URL": "O064",
    "invalid redirect encoding": "O065",
    "invalid redirect host": "O066",
    "invalid redirect size": "O067",
    "invalid series": "O068",
    "invalid source SHA": "O069",
    "invalid transport charge": "O070",
    "inventory changed during pagination": "O071",
    "inventory exceeds bound or lacks total": "O072",
    "job inventory identity drift": "O073",
    "job/check mapping unavailable or wrong origin": "O074",
    "known case acquired a new/replaced run": "C056",
    "local credential read exceeded bound": "O075",
    "local credential read timed out": "O076",
    "local gh credential unavailable": "O077",
    "local redirect refused": "O078",
    "log encoding unsupported": "O079",
    "log job/check not previously authenticated": "O080",
    "log privacy refused; hash only preserved": "O081",
    "metadata redirect refused": "O082",
    "new post-stable successful-control representation unsupported": "C057",
    "new/replaced/parallel role identities, including reused IDs; unsupported": "O083",
    "normal cancellation case/budget refused": "C058",
    "numeric redirect host refused": "O084",
    "observation run/case mismatch": "C059",
    "observer custody schema mismatch; do not mix collector versions": "O085",
    "observer source changed since series initialization": "O086",
    "operator account mismatch": "C060",
    "owned run is not freshly active": "C061",
    "phase label already used": "O087",
    "positive executing runner evidence unavailable": "O088",
    "positive fixed hold step unavailable": "O089",
    "positive preparation inventory disposition unavailable": "O090",
    "positive success job/check evidence unavailable": "O091",
    "post-stable control observation contradicts terminal inventory": "C062",
    "post-terminal observation replaced a shared role identity": "O092",
    "pre-cancellation active-run inventory unavailable": "O093",
    "preexisting run refused": "C063",
    "preflight already exhausted series clock": "O094",
    "preflight clock must have UTC offset": "O095",
    "preflight/normal-work budget unavailable": "C064",
    "preparation artifact identity/digest changed": "O096",
    "preparation inventory and fresh job/check observation contradict": "O097",
    "preparation inventory identity is not unique": "O098",
    "preparation read did not begin after dependent entry observation": "O099",
    "preparation-to-dependent control witness linkage missing": "C065",
    "previous artifact disappeared from complete inventory": "O100",
    "previous artifact identity/digest changed": "O101",
    "prior run must be uniquely bound": "C066",
    "probe workflow identity mismatch": "C067",
    "provider response is not an object": "O102",
    "provider returned non-success HTTP status": "O103",
    "redirect contains unsafe control or encoding": "O104",
    "redirect is not safe HTTPS": "O105",
    "repository identity unavailable or changed": "O106",
    "repository/default branch mismatch": "C068",
    "request wall-clock deadline exhausted": "O107",
    "required authenticated observation missing": "C069",
    "required terminal control missing": "C070",
    "response exceeds byte bound": "O108",
    "retention cases differ": "O109",
    "retention runs differ": "O110",
    "run already bound": "O111",
    "run correlation absent or ambiguous; do not redispatch": "C071",
    "run creation is outside the recorded dispatch attempt window": "C072",
    "run details identity drift": "O112",
    "run discovery incomplete or exceeds one-page bound": "C073",
    "run metadata must be an object": "O113",
    "run polling status unsupported": "O114",
    "run/attempt identity drift": "O115",
    "run/case is not explicitly bound": "O116",
    "same preparation archive was not independently verified": "O117",
    "same preparation artifact absent from one inventory": "O118",
    "separate operator-approved durable custody required": "C074",
    "series already registered; budget reset refused": "O119",
    "series deadline exhausted": "O120",
    "series request budget exhausted": "O121",
    "series/case run correlation unavailable": "O122",
    "server creation timestamp unavailable": "C075",
    "source binding contains unknown fields": "O123",
    "source binding must be an object": "O124",
    "source catalog role identity unavailable or inconsistent": "O125",
    "source commit mismatch": "C076",
    "source directory identity mismatch": "C077",
    "source manifest contains unknown fields": "O126",
    "source manifest exceeds bound": "O127",
    "source manifest identity mismatch": "O128",
    "source manifest must be an object": "O129",
    "source mode mismatch": "O130",
    "source tree unavailable or truncated": "C078",
    "source uploader binding mismatch": "O131",
    "source workflow catalog mismatch": "O132",
    "spaced stable successful-control inventory required": "C079",
    "stable run/source/check role linkage invalid": "O133",
    "successful A preparation replaced the held incumbent": "C080",
    "successful control job/check authentication or inventory binding missing": "C081",
    "successful expanded role unobserved": "O134",
    "successful prerequisite identity was replaced": "O135",
    "successful-control representation identity changed": "C082",
    "successful-control representation inventory changed": "C083",
    "successful-control role changed across stable inventories": "C084",
    "successful-control role inventory is empty or duplicated": "C085",
    "successful-control source/role/disposition unsupported": "C086",
    "terminal reread allowance exhausted": "O136",
    "unbound artifact response": "O137",
    "unbound check response": "O138",
    "unbound job response": "O139",
    "unbound series/case response": "O140",
    "unexpected archive members": "O141",
    "unexpected successful HTTP status": "O142",
    "unique successful expanded control role required": "C087",
    "unknown job catalog shape; complete raw inventory preserved": "O143",
    "unrelated new probe run; stop without controlling it": "C088",
    "unreserved effect refused": "C089",
    "unsupported server timestamp precision": "C090",
    "v5 controller custody schema mismatch": "V4C009",
    "v5 controller refuses older custody; no migration": "V4C010",
    "v5 draft exact source catalog mismatch": "V4O087",
    "verified control archive bytes absent from custody": "C091",
    "verified control archive contradicts complete inventory": "C092",
    "verified control archive run/case binding missing": "C093",
    "witness archive member exceeds bound": "O144",
    "witness must be an object": "O145",
    "witness schema or exact context binding mismatch": "O146",
    "workflow identity unavailable or changed": "O147",
    "workflow tree incomplete": "C094",
}
STRUCTURAL_KEYS = (
    "app",
    "check_record",
    "head_repository",
    "job_record",
    "object",
    "repository",
    "tree",
    "workflow_run",
    "workflows",
)
REFUSAL_CODES.update(
    {
        "required " + key + " metadata must be an object": "STRUCTURE_" + key.upper()
        for key in STRUCTURAL_KEYS
    }
)
REFUSAL_CODES.update(
    {
        "normal discovery cannot resume after abort": "DISCOVERY_ABORTED",
        "legacy custody refused; initialize a new series": "LEGACY_CUSTODY",
        "source descriptor keys mismatch": "SOURCE_DESCRIPTOR_KEYS",
        "reviewed descriptor digest mismatch": "SOURCE_DESCRIPTOR_DIGEST",
        "inventory read requires a scoped reservation": "DISCOVERY_UNRESERVED",
        "preflight inventory cannot be reused after control or refusal": "PREFLIGHT_REUSE",
        "discovery inventory reservation unavailable": "DISCOVERY_RESERVATION_INVALID",
        "discovery report exceeds bound": "DISCOVERY_REPORT_SIZE",
    }
)

REFUSAL_CODES.update(
    {
        "required actor metadata must be an object": "STRUCTURE_ACTOR",
        "required triggering_actor metadata must be an object": "STRUCTURE_TRIGGERING_ACTOR",
        "job name metadata must be a bounded string": "STRUCTURE_JOB_NAME",
    }
)

REFUSAL_CODES.update(
    {
        "terminal capture requires a unique sample label": "TERMINAL_SAMPLE_LABEL",
        "terminal capture already stable; no third read": "TERMINAL_ALREADY_STABLE",
        "first terminal response cannot be backfilled": "TERMINAL_BACKFILL_REFUSED",
        "spaced stable terminal capture required": "TERMINAL_CADENCE",
        "terminal capture status contradicted": "TERMINAL_STATUS_DRIFT",
        "terminal capture inventory unsupported": "TERMINAL_INVENTORY_UNSUPPORTED",
        "terminal capture label already bound": "TERMINAL_ALREADY_BOUND",
        "terminal run response custody unavailable": "TERMINAL_RESPONSE_CUSTODY",
    }
)

REFUSAL_CODES.update(
    {
        "malformed server creation timestamp": "SERVER_CREATION_TIMESTAMP_MALFORMED",
        "malformed local dispatch attempt timestamp": "DISPATCH_TIMESTAMP_MALFORMED",
    }
)


def refusal_code(error: BaseException | None) -> str:
    code = "UNCLASSIFIED_EXCEPTION"
    if type(error) is load_observer().Inconclusive:
        code = "UNCLASSIFIED_REFUSAL"
    if (
        isinstance(error, (load_observer().Inconclusive, ValueError))
        and len(error.args) == 1
        and type(error.args[0]) is str
    ):
        code = REFUSAL_CODES.get(error.args[0], code)
    return code


PHASE_CODES = frozenset(
    (
        "CONTROLLER_REQUEST",
        "CONTROLLER_GET",
        "SOURCE_HEAD",
        "DISCOVERY_LIST",
        "PREFLIGHT",
        "OBSERVATION_LOOKUP",
        "SNAPSHOT_LOOKUP",
        "SUCCESSFUL_CONTROL",
        "DISPATCH_GUARD",
        "RESERVED_EFFECT",
        "DISPATCH",
        "DISPATCH_WINDOW",
        "DISCOVERY",
        "DURABLE_E_READBACK",
        "CANCELLATION",
        "DISCOVERY_RECEIPT",
        "DISCOVERY_READ_ALLOWANCE",
        "DISCOVERY_NEW_RUN_GUARD",
        "DISCOVERY_NEW_RUN_IDENTITY",
        "DISCOVERY_KNOWN_RUN_GUARD",
        "DISCOVERY_TARGET_CORRELATION",
        "DISCOVERY_TARGET_IDENTITY",
        "DISCOVERY_TARGET_OWNERSHIP",
        "DISCOVERY_BIND_CUSTODY",
        "DISCOVERY_EXACT_TARGET_READ",
        "DISCOVERY_FINAL_RECORD",
        "CONTROLLER_INITIALIZATION",
    )
)
DISCOVERY_FACT_KEYS = frozenset(
    (
        "discovery_read_number",
        "own_title_match_count",
        "target_title_match_count",
        "target_identity_validated",
        "target_window_validated",
        "target_bound",
        "exact_target_authenticated",
        "normal_inventory_checks_required",
    )
)


def diagnostic_phase(phase: str) -> Callable[..., Any]:
    """Annotate existing operations without changing predicates, requests or custody."""

    def decorate(function: Callable[..., Any]) -> Callable[..., Any]:
        @functools.wraps(function)
        def annotated(self: Controller, *args: object, **kwargs: object) -> object:
            outermost = self._diagnostic_depth == 0
            if outermost:
                self._diagnostic_failure_phase = None
                self._diagnostic_last_phase = phase
                self._discovery_facts = {}
                self._discovery_context = None
            previous = self._diagnostic_phase
            self._diagnostic_phase = phase
            self._diagnostic_depth += 1
            try:
                return function(self, *args, **kwargs)
            except Exception:
                if self._diagnostic_failure_phase is None:
                    self._diagnostic_failure_phase = self._diagnostic_phase
                raise
            finally:
                self._diagnostic_depth -= 1
                self._diagnostic_phase = previous

        return annotated

    return decorate


def discovery_accounting(function: Callable[..., Any]) -> Callable[..., Any]:
    @functools.wraps(function)
    def accounted(self: Controller, case: str, diagnostic: bool = False) -> object:
        before = self.e.state["requests"]
        reads = self.e.state["discovery_reads"].get(case, 0)
        files = len(self.e.state["files"])
        self._discovery_context = {
            "global_before": before,
            "global_after": before,
            "global_delta": 0,
            "reads_before": reads,
            "reads_after": reads,
            "reads_delta": 0,
            "reads_remaining": 3 - reads,
            "diagnostic_only": diagnostic,
            "expected_source_sha": self.source,
            "observed_source_sha": None,
            "expected_repository": REPOSITORY,
            "observed_repository_matches": None,
            "expected_workflow_id": self.e.state["workflow_id"],
            "observed_workflow_id": None,
            "expected_attempt": 1,
            "expected_event": "workflow_dispatch",
            "expected_ref": "main",
            "observed_run_id": None,
            "observed_created_at": None,
            "workflow_path_matches": None,
            "window_matches": None,
            "observed_attempt": None,
            "expected_actor_id": self.e.state["actor"]["id"],
            "observed_actor_id": None,
            "expected_triggering_actor_id": self.e.state["triggering_actor"]["id"],
            "observed_triggering_actor_id": None,
            "event_matches": None,
            "ref_matches": None,
            "evidence_refs": [],
            "identity_observations": {"inventory": None, "exact": None},
            "observed_identity_provenance": None,
        }
        error = None
        try:
            return function(self, case, diagnostic)
        except BaseException as caught:
            error = caught
            self.c["aborted"] = True
            self._diagnostic_failure_phase = (
                self._diagnostic_failure_phase or self._diagnostic_phase
            )
            raise
        finally:
            self._inventory_permit = None
            after = self.e.state["requests"]
            count = self.e.state["discovery_reads"].get(case, 0)
            self._discovery_context.update(
                {
                    "global_after": after,
                    "global_delta": after - before,
                    "reads_after": count,
                    "reads_delta": count - reads,
                    "reads_remaining": 3 - count,
                    "evidence_refs": [
                        {key: item[key] for key in ("path", "bytes", "sha256")}
                        for item in self.e.state["files"][files:][-8:]
                    ],
                }
            )
            report = self.diagnostics(error)
            report["case"] = case
            reports = self.e.state.setdefault("discovery_reports", [])
            self.o.require(
                len(json.dumps(report).encode()) <= 8192, "discovery report exceeds bound"
            )
            reports.append(report)
            self.e.persist()

    return accounted


def load_observer() -> ModuleType:
    from . import provider_probe_observer

    return provider_probe_observer


class Controller:
    def __init__(self, observer: ModuleType, custody: str | Path) -> None:
        self._diagnostic_phase = "CONTROLLER_INITIALIZATION"
        self._diagnostic_last_phase = "CONTROLLER_INITIALIZATION"
        self._diagnostic_failure_phase = None
        self._diagnostic_depth = 0
        self._discovery_facts = {}
        self._discovery_context = None
        self._inventory_permit: dict[str, Any] | None = None
        self.o = observer
        self.e = observer.Evidence(custody)
        self.source = self.e.state["source_sha"]
        self.observer_hash = self.e.state["observer_sha256"]
        self.source_hash = hashlib.sha256(Path(__file__).read_bytes()).hexdigest()
        self.o.require(
            not any(key in self.e.state for key in ("controller_v1", "controller_v4")),
            "v5 controller refuses older custody; no migration",
        )
        self.c = self.e.state.setdefault(
            "controller_v5",
            {
                "schema": CONTROLLER_SCHEMA,
                "sha256": self.source_hash,
                "effects": {},
                "discovered": {},
                "aborted": False,
            },
        )
        self.o.require(
            self.c.get("schema") == CONTROLLER_SCHEMA, "v5 controller custody schema mismatch"
        )
        self.o.require(
            self.c["sha256"] == self.source_hash, "controller bytes changed during series"
        )
        if self.e.state.get("normal_aborted"):
            self.c["aborted"] = True
        if any(record.get("state") != "accepted" for record in self.c["effects"].values()):
            self.c["aborted"] = True  # crash/ambiguity cannot silently resume normal effects
        self.e.persist()
        self.token = None
        self.opener = urllib.request.build_opener(
            urllib.request.ProxyHandler({}), observer.NoRedirect()
        )

    def diagnostics(self, error: BaseException | None = None) -> dict[str, Any]:
        # The exception argument is compared only to frozen literal messages; never stringified.
        code = "NONE" if error is None else refusal_code(error)
        phase = self._diagnostic_failure_phase or self._diagnostic_last_phase
        result = {
            "diagnostic_version": 2,
            "phase_code": phase if phase in PHASE_CODES else "UNKNOWN_PHASE",
            "refusal_code": code,
        }
        facts = {
            key: value
            for key, value in self._discovery_facts.items()
            if key in DISCOVERY_FACT_KEYS
            and (type(value) is bool or (type(value) is int and 0 <= value <= 100))
        }
        if facts:
            result["own_series_discovery_facts"] = facts
        if self._discovery_context is not None:
            result["discovery_accounting"] = self._discovery_context
        result["requests"] = self.e.state["requests"]
        return result

    def label(self, purpose: str) -> str:
        return "controller-" + purpose + "-" + str(len(self.e.state["files"]) + 1)

    @diagnostic_phase("CONTROLLER_REQUEST")
    def request(
        self,
        method: str,
        path: str,
        payload: dict[str, Any] | None = None,
        reservation: str | None = None,
    ) -> tuple[int, bytes]:
        """Only private closed-route GETs and an already-spent exact effect may enter."""
        allowed_get = (
            path in {"/user", PREFIX, PREFIX + "/git/ref/heads/main"}
            or re.fullmatch(re.escape(PREFIX) + r"/git/(?:commits|trees|blobs)/[a-f0-9]{40}", path)
            or path
            in {
                PREFIX + "/actions/workflows/ci.yml",
                PREFIX + "/actions/workflows/recovery-cancellation-provider-probe.yml",
                PREFIX
                + "/actions/workflows/"
                + str(self.e.state["workflow_id"])
                + "/runs?per_page=100&page=1",
            }
            or path == PREFIX + "/actions/runs/" + str(self.c.get("main_ci_run"))
        )
        if method == "GET":
            self.o.require(
                allowed_get and payload is None and reservation is None, "GET route refused"
            )
            inventory_path = (
                PREFIX
                + "/actions/workflows/"
                + str(self.e.state["workflow_id"])
                + "/runs?per_page=100&page=1"
            )
            if path == inventory_path:
                permit, self._inventory_permit = self._inventory_permit, None
                self.o.require(
                    isinstance(permit, dict), "inventory read requires a scoped reservation"
                )
                permit = cast(dict[str, Any], permit)
                if permit.get("kind") == "preflight":
                    self.o.require(
                        not self.c.get("preflight")
                        and not self.c["effects"]
                        and not self.c["aborted"]
                        and not self.e.state["runs"],
                        "preflight inventory cannot be reused after control or refusal",
                    )
                else:
                    self.o.require(
                        permit.get("kind") == "discovery"
                        and self.e.state["discovery_reads"].get(permit.get("case"))
                        == permit.get("ordinal")
                        and 1 <= permit.get("ordinal", 0) <= 3
                        and (not self.c["aborted"] or permit.get("diagnostic") is True),
                        "discovery inventory reservation unavailable",
                    )
            self.e.reserve_read("controller:" + path)
        else:
            self.o.require(
                method == "POST" and reservation in self.c["effects"], "effect route refused"
            )
            record = self.c["effects"][reservation]
            self.o.require(
                record["path"] == path
                and record["payload"] == payload
                and record["state"] == "spent-before-transport",
                "unreserved effect refused",
            )
        self.e.charge(1, "controller-" + method + "-" + path)
        if self.token is None:
            self.token = self.o.token_from_gh()
        headers = {
            "Authorization": "Bearer " + self.token,
            "Accept": "application/vnd.github+json",
            "Content-Type": "application/json",
            "X-GitHub-Api-Version": "2022-11-28",
            "User-Agent": "boxferry-provider632-root-controller-draft-v1",
        }
        body = None if payload is None else json.dumps(payload, sort_keys=True).encode()
        request = urllib.request.Request(API + path, data=body, headers=headers, method=method)
        limit = self.o.MAX_JSON if method == "GET" else 16384
        before = self.o.utc()
        if method == "POST":
            self.c["effects"][reservation]["transport_started_utc"] = before
            self.e.persist()
        try:
            with self.o.hard_request_deadline(min(15, self.o.MAX_SECONDS - self.e.elapsed())):
                try:
                    response = self.opener.open(request, timeout=15)
                except urllib.error.HTTPError as error:
                    response = error
                with response:
                    status = response.status
                    length = response.headers.get("Content-Length")
                    self.o.require(
                        length is None or (length.isdecimal() and int(length) <= limit),
                        "controller response exceeds bound",
                    )
                    raw = response.read(limit + 1)
                    self.o.require(len(raw) <= limit, "controller response exceeds bound")
        except (
            OSError,
            urllib.error.URLError,
            http.client.HTTPException,
            ValueError,
            UnicodeError,
        ):
            raise self.o.Inconclusive(
                "controller transport outcome unavailable; never retry effect"
            ) from None
        finally:
            if method == "POST":
                ended = self.o.utc()
                self.c["effects"][reservation]["transport_end_utc"] = ended
                self.c["effects"][reservation]["after_utc"] = ended
                self.e.persist()
        summary = {
            "method": method,
            "path": path,
            "status": status,
            "before_utc": before,
            "after_utc": self.o.utc(),
            "bytes": len(raw),
            "sha256": hashlib.sha256(raw).hexdigest(),
        }
        # Discovery/account/source GETs retain bounded hashes, not unrelated raw account/run fields.
        self.e.save(
            self.label("response-summary"), (json.dumps(summary, sort_keys=True) + "\n").encode()
        )
        if method == "POST":
            suspicious = re.search(
                rb"(?i)(authorization\s*:|bearer\s+|https?://[^\s<>]+\?|github_pat_|gh[pousr]_)",
                raw,
            )
            self.o.require(
                not suspicious and self.token.encode() not in raw,
                "controller response privacy refusal; spent effect remains consumed",
            )
            self.e.save(self.label("effect-response"), raw)
        self.o.require(status not in self.o.REDIRECTS, "controller redirects refused")
        return cast(int, status), raw

    @diagnostic_phase("CONTROLLER_GET")
    def get(self, path: str) -> dict[str, Any]:
        status, raw = self.request("GET", path)
        self.o.require(status == 200, "controller GET unavailable")
        value = self.o.decode(raw)
        self.o.require(isinstance(value, dict), "controller GET object unavailable")
        return value

    @diagnostic_phase("SOURCE_HEAD")
    def head(self) -> None:
        value = self.get(PREFIX + "/git/ref/heads/main")
        obj = self.o.object_field(value, "object")
        self.o.require(
            value.get("ref") == "refs/heads/main"
            and obj.get("type") == "commit"
            and obj.get("sha") == self.source,
            "default main moved or source identity unavailable",
        )

    @diagnostic_phase("DISCOVERY_LIST")
    def run_list(self) -> list[dict[str, Any]]:
        path = (
            PREFIX
            + "/actions/workflows/"
            + str(self.e.state["workflow_id"])
            + "/runs?per_page=100&page=1"
        )
        value = self.get(path)
        rows, total = value.get("workflow_runs"), value.get("total_count")
        self.o.require(
            type(total) is int
            and 0 <= total <= 100
            and isinstance(rows, list)
            and len(rows) == total
            and all(isinstance(row, dict) for row in rows),
            "run discovery incomplete or exceeds one-page bound",
        )
        ids = [self.o.positive(row.get("id")) for row in rows]
        self.o.require(len(ids) == len(set(ids)), "duplicate discovery identity")
        return rows

    @diagnostic_phase("PREFLIGHT")
    def preflight(self, ci_run: int) -> None:
        self.o.require(
            not self.c["aborted"]
            and not self.c.get("preflight")
            and not self.c["effects"]
            and not self.e.state["runs"],
            "cannot attach controller to existing runs or reset preflight",
        )
        self.o.require(
            self.o.positive(ci_run) == self.e.state["source_descriptor"]["main_ci_run"],
            "controller main CI identity mismatch",
        )
        self.c["main_ci_run"] = ci_run
        self.e.persist()
        user = self.get("/user")
        for key in ("actor", "triggering_actor"):
            self.o.require(
                user.get("login") == self.e.state[key]["login"]
                and user.get("id") == self.e.state[key]["id"],
                "operator account mismatch",
            )
        repository = self.get(PREFIX)
        self.o.require(
            repository.get("full_name") == REPOSITORY
            and repository.get("default_branch") == "main",
            "repository/default branch mismatch",
        )
        self.head()
        workflow = self.get(PREFIX + "/actions/workflows/recovery-cancellation-provider-probe.yml")
        self.o.require(
            workflow.get("id") == self.e.state["workflow_id"]
            and workflow.get("path") == WORKFLOW
            and workflow.get("name") == "629 recovery cancellation provider probe"
            and workflow.get("state") == "active",
            "probe workflow identity mismatch",
        )
        ci_workflow = self.get(PREFIX + "/actions/workflows/ci.yml")
        ci = self.get(PREFIX + "/actions/runs/" + str(ci_run))
        self.o.positive(ci_workflow.get("id"))
        self.o.require(
            ci.get("id") == self.e.state["source_descriptor"]["main_ci_run"]
            and ci_workflow.get("path") == ".github/workflows/ci.yml"
            and ci.get("workflow_id") == ci_workflow.get("id")
            and ci.get("head_sha") == self.source
            and ci.get("head_branch") == "main"
            and ci.get("event") == "push"
            and ci.get("status") == "completed"
            and ci.get("conclusion") == "success"
            and self.o.object_field(ci, "repository").get("full_name") == REPOSITORY,
            "exact source main CI has not succeeded",
        )
        commit = self.get(PREFIX + "/git/commits/" + self.source)
        self.o.require(commit.get("sha") == self.source, "source commit mismatch")
        tree_id = self.o.sha(self.o.object_field(commit, "tree").get("sha"))
        for directory in (".github", "workflows"):
            tree = self.get(PREFIX + "/git/trees/" + tree_id)
            self.o.require(
                tree.get("sha") == tree_id and tree.get("truncated") is False,
                "source tree unavailable or truncated",
            )
            matches = [
                row
                for row in tree.get("tree", [])
                if isinstance(row, dict) and row.get("path") == directory
            ]
            self.o.require(
                len(matches) == 1
                and matches[0].get("type") == "tree"
                and matches[0].get("mode") == "040000",
                "source directory identity mismatch",
            )
            tree_id = self.o.sha(matches[0].get("sha"))
        tree = self.get(PREFIX + "/git/trees/" + tree_id)
        self.o.require(
            tree.get("sha") == tree_id and tree.get("truncated") is False,
            "workflow tree incomplete",
        )
        for path, approved in self.e.state["source_descriptor"]["workflows"].items():
            matches = [
                row
                for row in tree.get("tree", [])
                if isinstance(row, dict) and row.get("path") == Path(path).name
            ]
            self.o.require(
                len(matches) == 1
                and matches[0].get("type") == "blob"
                and matches[0].get("mode") == "100644"
                and matches[0].get("sha") == approved["blob"],
                "approved workflow mode/blob mismatch",
            )
            blob = self.get(PREFIX + "/git/blobs/" + approved["blob"])
            self.o.require(
                blob.get("sha") == approved["blob"] and blob.get("encoding") == "base64",
                "blob unavailable",
            )
            data = base64.b64decode("".join(blob["content"].split()), validate=True)
            self.o.require(
                len(data) <= 65536
                and len(data) == blob.get("size")
                and hashlib.sha256(data).hexdigest() == approved["sha256"]
                and hashlib.sha1(b"blob " + str(len(data)).encode() + b"\0" + data).hexdigest()
                == approved["blob"],
                "approved workflow bytes mismatch",
            )
        self._inventory_permit = {"kind": "preflight"}
        rows = self.run_list()
        self.o.require(
            all(row.get("status") == "completed" for row in rows), "another probe run is active"
        )
        self.c["preflight_ids"] = [row["id"] for row in rows]
        self.c["preflight"] = True
        self.c["preflight_evidence"] = {
            "source_sha": self.source,
            "workflow_id": self.e.state["workflow_id"],
            "main_ci_run": ci_run,
            "approved_blobs": self.e.state["source_descriptor"]["workflows"],
            "actor": self.e.state["actor"],
            "utc": self.o.utc(),
        }
        self.e.persist()

    @diagnostic_phase("OBSERVATION_LOOKUP")
    def observation(self, label: str, case: str) -> dict[str, Any]:
        value = self.e.state["observations"].get(label)
        self.o.require(isinstance(value, dict), "required authenticated observation missing")
        self.o.require(
            value["case"] == case
            and value["run"] == self.e.state["runs"][case]["id"]
            and self.c["discovered"].get(case) == value["run"],
            "observation run/case mismatch",
        )
        return value

    @diagnostic_phase("SNAPSHOT_LOOKUP")
    def snapshot(self, label: str, case: str, conclusion: object) -> dict[str, Any]:
        value = self.e.state["snapshots"].get(label)
        self.o.require(isinstance(value, dict), "required terminal control missing")
        self.o.require(
            value["case"] == case
            and value["run"] == self.e.state["runs"][case]["id"]
            and self.c["discovered"].get(case) == value["run"]
            and value["run_status"] == "completed"
            and value["run_conclusion"] == conclusion,
            "required terminal control missing",
        )
        return value

    @diagnostic_phase("SUCCESSFUL_CONTROL")
    def successful_control(self, case: str) -> dict[str, Any]:
        run = self.e.state["runs"][case]["id"]
        first = self.snapshot(case + "-terminal", case, "success")
        stable = self.snapshot(case + "-terminal-stable", case, "success")
        self.o.require(
            stable["elapsed"] - first["elapsed"] >= 5,
            "spaced stable successful-control inventory required",
        )
        identity = (
            "id",
            "name",
            "catalog_name_shape",
            "run_id",
            "run_attempt",
            "head_sha",
            "check_run_url",
        )
        leaf_rows = {}
        for snapshot in (first, stable):
            self.o.require(
                isinstance(snapshot.get("jobs"), list)
                and isinstance(snapshot.get("artifacts"), list),
                "complete successful-control inventories unavailable",
            )
            rows = snapshot["jobs"]
            self.o.require(
                rows and len({row["id"] for row in rows}) == len(rows),
                "successful-control role inventory is empty or duplicated",
            )
            for row in rows:
                self.o.positive(row.get("id"))
                name = row.get("name")
                self.o.require(
                    isinstance(name, str)
                    and name in self.o.CATALOG
                    and row.get("catalog_name_shape") == list(self.o.CATALOG[name])
                    and row.get("run_id") == run
                    and row.get("run_attempt") == 1
                    and row.get("head_sha") == self.source
                    and row.get("status") == "completed"
                    and row.get("conclusion") == "success"
                    and isinstance(row.get("check_run_url"), str)
                    and re.fullmatch(
                        re.escape(API + PREFIX) + r"/check-runs/[1-9][0-9]*", row["check_run_url"]
                    ),
                    "successful-control source/role/disposition unsupported",
                )
            for role in ("prepare", "effect"):
                found = [
                    row for row in rows if row["catalog_name_shape"] == [role, "expanded-leaf"]
                ]
                self.o.require(len(found) == 1, "unique successful expanded control role required")
                if snapshot is first:
                    leaf_rows[role] = found[0]
                else:
                    self.o.require(
                        all(found[0].get(key) == leaf_rows[role].get(key) for key in identity),
                        "successful-control role changed across stable inventories",
                    )
            self.o.require(
                {row["id"] for row in rows} == {row["id"] for row in first["jobs"]},
                "successful-control representation inventory changed",
            )
            for row in rows:
                prior = next(item for item in first["jobs"] if item["id"] == row["id"])
                self.o.require(
                    all(row.get(key) == prior.get(key) for key in identity),
                    "successful-control representation identity changed",
                )
        stable_ids = {row["id"]: row for row in stable["jobs"]}
        for observation in self.e.state["observations"].values():
            if (
                observation.get("case") == case
                and observation.get("run") == run
                and observation.get("started_elapsed", -1) > stable["elapsed"]
            ):
                self.o.require(
                    observation.get("job") in stable_ids,
                    "new post-stable successful-control representation unsupported",
                )
                current = stable_ids[observation["job"]]
                job = self.o.object_field(observation, "job_record")
                check = self.o.object_field(observation, "check_record")
                self.o.require(
                    observation.get("catalog_name_shape") == current["catalog_name_shape"]
                    and all(
                        current.get(field) == job.get(field)
                        for field in (
                            "id",
                            "name",
                            "run_id",
                            "run_attempt",
                            "head_sha",
                            "check_run_url",
                            "status",
                            "conclusion",
                        )
                    )
                    and check.get("status") == "completed"
                    and check.get("conclusion") == "success",
                    "post-stable control observation contradicts terminal inventory",
                )
        roles = {}
        for role, row in leaf_rows.items():
            candidates = [
                value
                for value in self.e.state["observations"].values()
                if value.get("case") == case
                and value.get("run") == run
                and value.get("job") == row["id"]
                and value.get("catalog_name_shape") == [role, "expanded-leaf"]
                and value.get("started_elapsed", -1) > stable["elapsed"]
            ]
            self.o.require(
                candidates,
                "authenticated successful control leaves after stable inventory required",
            )
            observation = max(candidates, key=lambda value: value["elapsed"])
            self.o.successful(observation, role)
            job, check = observation["job_record"], observation["check_record"]
            self.o.require(
                all(
                    key in job and row.get(key) == job[key]
                    for key in (
                        "id",
                        "name",
                        "run_id",
                        "run_attempt",
                        "head_sha",
                        "check_run_url",
                        "status",
                        "conclusion",
                    )
                )
                and job["id"] == observation["job"]
                and self.o.positive(job.get("runner_id"))
                and check.get("id") == observation["check"]
                and check.get("name") == job["name"]
                and check.get("head_sha") == self.source
                and self.o.object_field(check, "app").get("id") == 15368
                and self.o.object_field(check, "app").get("slug") == "github-actions"
                and job["check_run_url"]
                == API + PREFIX + "/check-runs/" + str(observation["check"])
                and check.get("details_url")
                == f"https://github.com/{REPOSITORY}/actions/runs/{run}/job/{job['id']}",
                "successful control job/check authentication or inventory binding missing",
            )
            roles[role] = observation
        if case == "A":
            held = self.observation("A-after-B", "A")
            self.o.require(
                (roles["prepare"]["job"], roles["prepare"]["check"])
                == (held["job"], held["check"]),
                "successful A preparation replaced the held incumbent",
            )
        verified = self.e.state["runs"][case].get("verified_artifacts", {})
        self.o.require(
            isinstance(verified, dict) and len(verified) == 2,
            "both verified successful-control archives required",
        )
        first_artifacts = {row["id"]: row for row in first["artifacts"]}
        stable_artifacts = {row["id"]: row for row in stable["artifacts"]}
        self.o.require(
            len(first_artifacts) == len(stable_artifacts) == 2
            and set(first_artifacts) == set(stable_artifacts),
            "complete stable control artifact inventory required",
        )
        witnessed = {}
        for key, record in verified.items():
            identifier = self.o.positive(record.get("artifact_id"))
            self.o.require(
                key == str(identifier)
                and identifier in stable_artifacts
                and record.get("run") == run
                and record.get("case") == case
                and record.get("outcome") == "OBSERVED",
                "verified control archive run/case binding missing",
            )
            artifact = stable_artifacts[identifier]
            self.e.validate_artifact(artifact, case, run)
            self.o.require(
                all(
                    artifact.get(field) == first_artifacts[identifier].get(field)
                    for field in ("id", "name", "size_in_bytes", "digest", "workflow_run")
                )
                and record.get("archive_sha256") == artifact["digest"][7:]
                and record.get("archive_bytes") == artifact["size_in_bytes"],
                "verified control archive contradicts complete inventory",
            )
            archived = [
                item
                for item in self.e.state["files"]
                if item["sha256"] == record["archive_sha256"]
                and item["path"].endswith("-archive.zip")
            ]
            self.o.require(archived, "verified control archive bytes absent from custody")
            indexed = archived[0]
            self.o.require(
                Path(indexed["path"]).name == indexed["path"],
                "invalid control archive custody path",
            )
            path = self.e.directory / indexed["path"]
            self.o.require(
                not path.is_symlink()
                and path.is_file()
                and path.stat().st_size == record["archive_bytes"] <= self.o.MAX_ARCHIVE,
                "control archive custody shape mismatch",
            )
            with path.open("rb") as stream:
                raw = stream.read(self.o.MAX_ARCHIVE + 1)
            # Recheck bytes against an already-verified record; this creates no new uploader
            # attribution.
            witness = self.o.verify_archive(
                raw, artifact, self.e.state, case, run, record["archive_sha256"]
            )
            self.o.require(
                witness == record.get("witness")
                and hashlib.sha256(self.o.witness_bytes(raw)).hexdigest()
                == record.get("witness_sha256"),
                "control witness differs from its verified bytes",
            )
            role = witness["role"]
            self.o.require(role not in witnessed, "duplicate control witness role")
            witnessed[role] = (record, witness)
        self.o.require(
            set(witnessed) == {"prepare", "effect"}, "both control witness roles required"
        )
        prep, effect = witnessed["prepare"], witnessed["effect"]
        self.o.require(
            effect[1]["preparation_artifact_id"] == str(prep[0]["artifact_id"])
            and effect[1]["preparation_artifact_digest"] == prep[0]["archive_sha256"]
            and effect[1]["preparation_marker"]
            == f"{self.o.SCHEMA}:{self.e.state['series']}:{run}:1:{self.source}"
            and effect[1]["caller_prerequisite_result"] == "success",
            "preparation-to-dependent control witness linkage missing",
        )
        return {
            "case": case,
            "run": run,
            "prepare_job": roles["prepare"]["job"],
            "effect_job": roles["effect"]["job"],
            "artifact_ids": sorted(stable_artifacts),
        }

    @diagnostic_phase("DISPATCH_GUARD")
    def dispatch_guard(self, case: str) -> None:
        if case == "A":
            self.o.require(not self.c["effects"] and not self.e.state["runs"], "A must be first")
        elif case in {"B", "C"}:
            label = "A-before-" + case
            incumbent = self.observation(label, "A")
            self.o.holding(incumbent, "prepare")
            self.o.require(
                0 <= self.e.elapsed() - incumbent["elapsed"] <= 5, "fresh A hold observation needed"
            )
            if case == "C":
                pending = self.e.state["snapshots"]["B-pending"]
                self.o.require(
                    pending["case"] == "B"
                    and pending["run"] == self.e.state["runs"]["B"]["id"]
                    and pending["run_status"] == "pending"
                    and pending["elapsed"] < incumbent["started_elapsed"],
                    "B pending before fresh A proof missing",
                )
        elif case == "D":
            self.successful_control("A")
            self.successful_control("C")
            self.o.guard_burst(
                self.e,
                SimpleNamespace(
                    before="A-before-C", after="A-after-B", pending="B-pending", victim="B-terminal"
                ),
            )
        else:
            self.snapshot("D-terminal", "D", "cancelled")
            self.o.require(
                self.c["effects"].get("cancel:D", {}).get("state") == "accepted",
                "D cancellation not accepted",
            )
            failed = self.observation("D-preparation-terminal", "D")
            held = self.observation("D-hold", "D")
            self.o.require(
                (failed["job"], failed["check"]) == (held["job"], held["check"])
                and failed["catalog_name_shape"] == ["prepare", "expanded-leaf"]
                and all(
                    failed[key].get("status") == "completed"
                    and failed[key].get("conclusion") in {"cancelled", "failure", "timed_out"}
                    for key in ("job_record", "check_record")
                ),
                "D positive failed preparation missing",
            )
            terminal = self.e.state["snapshots"]["D-terminal"]
            self.o.require(
                all(not row["name"].endswith("-effect-entry") for row in terminal["artifacts"])
                and all(
                    row.get("conclusion") == "skipped"
                    for row in terminal["jobs"]
                    if (row.get("catalog_name_shape") or [None])[0] == "effect"
                ),
                "D dependent entry contradiction or unsupported dependent disposition",
            )

    @diagnostic_phase("RESERVED_EFFECT")
    def effect(
        self,
        key: str,
        case: str,
        path: str,
        payload: dict[str, Any] | None,
        expected_status: object,
        kind: str,
    ) -> None:
        self.o.require(
            key not in self.c["effects"], "effect already spent; ambiguity never permits retry"
        )
        record = {
            "case": case,
            "kind": kind,
            "method": "POST",
            "path": path,
            "payload": payload,
            "source_sha": self.source,
            "series": self.e.state["series"],
            "before_utc": self.o.utc(),
            "before_elapsed": self.e.elapsed(),
            "state": "spent-before-transport",
        }
        self.c["effects"][key] = record
        # irreversible reservation before charge, credential lookup or HTTP attempt
        self.e.persist()
        try:
            status, raw = self.request("POST", path, payload, key)
            record.update(
                after_utc=record["transport_end_utc"],
                response_status=status,
                response_sha256=hashlib.sha256(raw).hexdigest(),
            )
            self.o.require(
                status == expected_status, "effect response unsupported; reservation remains spent"
            )
            record["state"] = "accepted"
            self.e.persist()
        except Exception:
            record.setdefault("after_utc", self.o.utc())
            record["state"] = "uncertain-or-refused-no-retry"
            self.c["aborted"] = True
            self.e.persist()
            raise
        self.e.save(
            self.label("effect-ledger"), (json.dumps(record, sort_keys=True) + "\n").encode()
        )

    @diagnostic_phase("DISPATCH")
    def dispatch(self, case: str) -> None:
        self.o.require(
            self.c.get("preflight")
            and not self.c["aborted"]
            and self.e.elapsed() < 1200
            and self.e.state["requests"] + 2 <= 373,
            "preflight/normal-work budget unavailable",
        )
        self.o.require(
            case not in self.e.state["runs"], "cannot dispatch/adopt an already-bound case"
        )
        self.head()
        self.dispatch_guard(case)
        previous = "ABCDE"[: "ABCDE".index(case)]
        self.o.require(
            all(
                letter in self.e.state["runs"]
                and self.c["discovered"].get(letter) == self.e.state["runs"][letter]["id"]
                for letter in previous
            ),
            "prior run must be uniquely bound",
        )
        payload = {
            "ref": "main",
            "inputs": {
                "case": case,
                "series": self.e.state["series"],
                "expected_source_sha": self.source,
            },
        }
        self.effect(
            "dispatch:" + case,
            case,
            PREFIX + "/actions/workflows/" + str(self.e.state["workflow_id"]) + "/dispatches",
            payload,
            204,
            "fixture-dispatch",
        )

    @diagnostic_phase("DISPATCH_WINDOW")
    def dispatch_window(self, record: dict[str, Any], created_text: str) -> None:
        self.o.require(isinstance(created_text, str), "server creation timestamp unavailable")
        match = re.fullmatch(
            r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.(\d{1,6}))?(?:Z|\+00:00)", created_text
        )
        self.o.require(match, "unsupported server timestamp precision")
        try:
            created = dt.datetime.fromisoformat(created_text.replace("Z", "+00:00"))
        except ValueError:
            raise self.o.Inconclusive("malformed server creation timestamp") from None
        start_text = record.get("transport_started_utc", record.get("before_utc"))
        end_text = record.get("after_utc")
        self.o.require(
            isinstance(start_text, str) and isinstance(end_text, str),
            "bounded dispatch attempt end unavailable",
        )
        try:
            start = dt.datetime.fromisoformat(start_text.replace("Z", "+00:00"))
            end = dt.datetime.fromisoformat(cast(str, end_text).replace("Z", "+00:00"))
        except ValueError:
            raise self.o.Inconclusive("malformed local dispatch attempt timestamp") from None
        self.o.require(
            start.utcoffset() == end.utcoffset() == dt.timedelta(0) and start <= end,
            "dispatch attempt UTC ordering invalid",
        )
        # A whole-second API value denotes [t,t+1s); fractional values use their stated precision.
        precision = dt.timedelta(
            microseconds=10 ** (6 - len(cast(re.Match[str], match).group(1) or ""))
        )
        self.o.require(
            created <= end and start - created < precision,
            "run creation is outside the recorded dispatch attempt window",
        )

    @diagnostic_phase("DISCOVERY")
    @discovery_accounting
    def discover(self, case: str, diagnostic: bool = False) -> bool:
        self._diagnostic_phase = "DISCOVERY_RECEIPT"
        self.o.require(
            not self.c["aborted"] or diagnostic, "normal discovery cannot resume after abort"
        )
        if diagnostic:
            self.c["aborted"] = True
            self.e.persist()
        record = self.c["effects"]["dispatch:" + case]
        if record.get("state") != "accepted":
            self.c["aborted"] = (
                True  # correlated ambiguity permits cleanup only, never normal execution
            )
            self.e.persist()
            self.o.require(
                record.get("transport_started_utc") and record.get("after_utc"),
                "bounded ambiguous dispatch attempt window unavailable",
            )
        self.o.require(
            diagnostic or case not in self.e.state["runs"], "case already bound; never replace run"
        )
        self._diagnostic_phase = "DISCOVERY_READ_ALLOWANCE"
        count = self.e.reserve_discovery(case)
        self._inventory_permit = {
            "kind": "discovery",
            "case": case,
            "ordinal": count + 1,
            "diagnostic": diagnostic,
        }
        self._discovery_facts = {
            "discovery_read_number": count + 1,
            "target_identity_validated": False,
            "target_window_validated": False,
            "target_bound": False,
            "exact_target_authenticated": False,
            "normal_inventory_checks_required": not self.c["aborted"],
        }
        rows = self.run_list()
        titles = {
            "629 provider probe " + self.e.state["series"] + " case " + letter for letter in "ABCDE"
        }
        target_title = "629 provider probe " + self.e.state["series"] + " case " + case
        self._discovery_facts.update(
            {
                "discovery_read_number": count + 1,
                "own_title_match_count": sum(
                    row.get("display_title") in titles
                    for row in rows
                    if type(row.get("display_title")) is str
                ),
                "target_title_match_count": sum(
                    row.get("display_title") == target_title for row in rows
                ),
                "target_identity_validated": False,
                "target_window_validated": False,
                "target_bound": False,
                "exact_target_authenticated": False,
                "normal_inventory_checks_required": not self.c["aborted"],
            }
        )
        own = [row for row in rows if row.get("display_title") == target_title]
        if len(own) == 1:
            self.discovery_identity(own[0])
        if not self.c["aborted"]:
            # Any newly observed unrelated concurrency event suspends normal controls.
            for row in rows:
                self._diagnostic_phase = "DISCOVERY_NEW_RUN_GUARD"
                if row["id"] in self.c["preflight_ids"]:
                    continue
                matched = [
                    letter
                    for letter in "ABCDE"
                    if row.get("display_title")
                    == "629 provider probe " + self.e.state["series"] + " case " + letter
                ]
                self.o.require(
                    len(matched) == 1 and "dispatch:" + matched[0] in self.c["effects"],
                    "unrelated new probe run; stop without controlling it",
                )
                self._diagnostic_phase = "DISCOVERY_NEW_RUN_IDENTITY"
                self.e.validate_run(row, matched[0], row["id"])
                self._diagnostic_phase = "DISCOVERY_KNOWN_RUN_GUARD"
                self.o.require(
                    matched[0] not in self.c["discovered"]
                    or self.c["discovered"][matched[0]] == row["id"],
                    "known case acquired a new/replaced run",
                )
        self._diagnostic_phase = "DISCOVERY_TARGET_CORRELATION"
        title = "629 provider probe " + self.e.state["series"] + " case " + case
        candidates = [row for row in rows if row.get("display_title") == title]
        if not candidates and count < 2:
            self.e.save(
                self.label("discovery-not-yet-" + case),
                b'{"outcome":"NOT_YET_OBSERVED; no effect retry"}\n',
            )
            return False
        self.o.require(
            len(candidates) == 1, "run correlation absent or ambiguous; do not redispatch"
        )
        value = candidates[0]
        self._diagnostic_phase = "DISCOVERY_TARGET_IDENTITY"
        self.e.validate_run(value, case, self.o.positive(value.get("id")))
        self._discovery_facts["target_identity_validated"] = True
        self._diagnostic_phase = "DISCOVERY_TARGET_OWNERSHIP"
        self.o.require(
            value["id"] not in self.c["preflight_ids"]
            and (
                diagnostic
                or value["id"] not in [row["id"] for row in self.e.state["runs"].values()]
            ),
            "preexisting run refused",
        )
        cast(dict[str, Any], self._discovery_context)["window_matches"] = False
        self.dispatch_window(record, value.get("created_at"))
        cast(dict[str, Any], self._discovery_context)["window_matches"] = True
        self._discovery_facts["target_window_validated"] = True
        if diagnostic:
            return True  # Correlation only; no binding or authority to resume normal controls.
        self._diagnostic_phase = "DISCOVERY_BIND_CUSTODY"
        self.e.state["runs"][case] = {"id": value["id"]}
        self.c["discovered"][case] = value["id"]
        self.e.persist()
        # Bind only one candidate, then read its exact authenticated run record too.
        self._discovery_facts["target_bound"] = True
        self._diagnostic_phase = "DISCOVERY_EXACT_TARGET_READ"
        self.e.last_run_response = None
        try:
            exact, _ = self.e.run(case, value["id"], self.label("exact-discovered-" + case))
        finally:
            response = self.e.last_run_response
            if isinstance(response, dict) and response["response"]["request_run_id"] == value["id"]:
                context = cast(dict[str, Any], self._discovery_context)
                context["identity_observations"]["exact"] = {
                    **response,
                    "processing_stage": "DISCOVERY_EXACT_TARGET_READ",
                }
                context.update(response["identity"])
                context["observed_identity_provenance"] = "exact"
        self._discovery_facts["exact_target_authenticated"] = True
        self._diagnostic_phase = "DISCOVERY_FINAL_RECORD"
        self.e.save(
            self.label("discovered-" + case), (json.dumps(exact, sort_keys=True) + "\n").encode()
        )
        return True

    def discovery_identity(self, value: dict[str, Any]) -> None:
        context = cast(dict[str, Any], self._discovery_context)
        facts = self.o.run_identity_facts(value)
        context.update(facts)
        context["observed_identity_provenance"] = "inventory"
        context["identity_observations"]["inventory"] = {
            "provenance": "inventory",
            "processing_stage": "DISCOVERY_INVENTORY_RESPONSE",
            "json_state": "object",
            "identity": facts,
        }

    @diagnostic_phase("DURABLE_E_READBACK")
    def durable_e_copy(self, directory: str | Path, proof_boundary: list[dict[str, Any]]) -> None:
        candidate = Path(directory)
        self.o.require(not candidate.is_symlink(), "E durable custody directory symlink refused")
        destination = candidate.resolve()
        self.o.require(
            destination != self.e.directory and not destination.is_relative_to(self.e.directory),
            "separate operator-approved durable custody required",
        )
        cut = self.e.state.get("e_v5", {}).get("pre_cancel_cut")
        self.o.require(
            cut
            and cut["observer_sha256"] == self.observer_hash
            and cut["controller_sha256"] == self.source_hash,
            "E exact helper-bound pre-cancel cut required",
        )
        self.o.require(
            proof_boundary and proof_boundary[: len(cut["files"])] == cut["files"],
            "E frozen cut is not a prefix of pre-cancel boundary",
        )
        started = self.e.elapsed()
        copied_path = destination / "state.json"
        self.o.require(
            not copied_path.is_symlink()
            and copied_path.is_file()
            and copied_path.stat().st_size <= self.o.MAX_JSON,
            "durable state file exceeds bound",
        )
        with copied_path.open("rb") as stream:
            copied_raw = stream.read(self.o.MAX_JSON + 1)
            os.fsync(stream.fileno())
        self.o.require(len(copied_raw) <= self.o.MAX_JSON, "durable state exceeds bound")
        copied = self.o.decode(copied_raw)
        self.o.require(
            copied.get("series") == self.e.state["series"]
            and copied.get("source_sha") == self.source
            and copied.get("observer_sha256") == self.observer_hash
            and copied.get("controller_v5", {}).get("sha256") == self.source_hash
            and copied.get("controller_v5", {}).get("schema") == CONTROLLER_SCHEMA
            and "cancel:E" not in copied["controller_v5"]["effects"]
            and copied.get("e_v5", {}).get("pre_cancel_cut") == cut
            and copied.get("files") == proof_boundary,
            "E durable copy identity cut or phase mismatch",
        )
        # Re-run the actual preparation ZIP/schema/provenance and execution proof on copied bytes.
        copied_evidence = self.o.Evidence(destination, read_only=True)
        proof = self.o.decode(
            self.o.e_bytes(
                self.o.e_pre_proof(
                    copied_evidence,
                    SimpleNamespace(prepare="E-prepare-pre-cancel", effect="E-effect-hold"),
                )
            )
        )
        self.o.require(
            all(cut.get(key) == value for key, value in proof.items()),
            "E copied mandatory pre-cancel proof incomplete or changed",
        )
        for row in proof_boundary:
            self.o.require(
                Path(row["path"]).name == row["path"], "durable proof index missing or changed"
            )
            path = destination / row["path"]
            self.o.require(
                not path.is_symlink()
                and path.is_file()
                and path.stat().st_size == row["bytes"]
                and row["bytes"] <= self.o.MAX_CUSTODY,
                "durable proof file shape mismatch",
            )
            with path.open("rb") as stream:
                data = stream.read(row["bytes"] + 1)
                os.fsync(stream.fileno())
            self.o.require(
                len(data) == row["bytes"] and hashlib.sha256(data).hexdigest() == row["sha256"],
                "durable readback mismatch",
            )
        # Environmental storage-class attestation is explicit; these syscalls do not prove power-
        # loss/WORM durability.
        attestation_path = destination / "storage-attestation.json"
        self.o.require(
            not attestation_path.is_symlink()
            and attestation_path.is_file()
            and attestation_path.stat().st_size <= 16384,
            "E reviewed storage attestation unavailable",
        )
        with attestation_path.open("rb") as stream:
            attestation_raw = stream.read(16385)
            os.fsync(stream.fileno())
        attestation = self.o.decode(attestation_raw)
        expected = {
            "schema": "boxferry.provider632.storage-attestation.v5",
            "series": self.e.state["series"],
            "source_sha": self.source,
            "observer_sha256": self.observer_hash,
            "controller_sha256": self.source_hash,
            "cut_sha256": cut["cut_sha256"],
            "root_reviewed": True,
            "storage_class": "operator-reviewed-durable-filesystem",
            "limits": "fsync-and-readback-only; no WORM or power-loss guarantee",
        }
        self.o.require(attestation == expected, "E reviewed storage attestation identity mismatch")
        descriptor = os.open(destination, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
        try:
            os.fsync(descriptor)
        finally:
            os.close(descriptor)
        self.c["E_durable_readback"] = {
            "directory": str(destination),
            "cut_sha256": cut["cut_sha256"],
            "state_sha256": hashlib.sha256(copied_raw).hexdigest(),
            "required_files": proof_boundary,
            "before_elapsed": started,
            "elapsed": self.e.elapsed(),
            "verified_utc": self.o.utc(),
            "file_fsync_count": len(proof_boundary) + 2,
            "directory_fsynced": True,
            "storage_attestation_sha256": hashlib.sha256(attestation_raw).hexdigest(),
            "storage_attestation": attestation,
        }
        self.e.persist()

        # Storage durability and fsync provenance remain an explicit root-reviewed environment
        # prerequisite.

    @diagnostic_phase("CANCELLATION")
    def cancel(
        self, case: str, run: int, cleanup: bool, durable: str | Path, reason: str | None = None
    ) -> None:
        proof_boundary = list(
            self.e.state["files"]
        )  # every pre-existing raw/normalized/log/controller record
        self.o.require(
            case in self.e.state["runs"]
            and self.e.state["runs"][case]["id"] == run
            and self.c["discovered"].get(case) == run
            and "dispatch:" + case in self.c["effects"],
            "cannot control an existing or unrelated run",
        )
        self.o.require(
            "cancel:" + case not in self.c["effects"], "cancellation already spent; never retry"
        )
        if cleanup:
            self.o.require(
                reason in {"source", "identity", "budget", "evidence", "deadline"},
                "explicit cleanup reason required",
            )
            self.c["abort_reason"] = reason
            self.c["aborted"] = True
            self.e.persist()
        else:
            self.o.require(
                case in {"D", "E"}
                and not self.c["aborted"]
                and self.e.elapsed() < 1200
                and self.e.state["requests"] + 3 <= 373,
                "normal cancellation case/budget refused",
            )
            self.head()
            if case == "D":
                self.o.holding(self.observation("D-hold", "D"), "prepare")
            else:
                self.o.require(
                    self.e.state.get("e_v5", {}).get("pre_cancel_cut"),
                    "E cut must be frozen and copied before cancellation invocation",
                )
                self.o.guard_e(
                    self.e, SimpleNamespace(prepare="E-prepare-pre-cancel", effect="E-effect-hold")
                )
                self.o.require(
                    self.e.state["snapshots"]["E-inventory-before"]["run_status"] == "in_progress",
                    "E pre-cancel full inventory required",
                )
                self.o.require(durable is not None, "durable E proof required before cancellation")
                self.durable_e_copy(durable, proof_boundary)
        # Fresh exact identity immediately before the only ordinary cancellation request.
        value, _ = self.e.run(case, run, self.label("fresh-owned-" + case))
        self.o.require(
            value.get("status") in {"queued", "requested", "waiting", "pending", "in_progress"},
            "owned run is not freshly active",
        )
        if not cleanup:
            label = "D-hold" if case == "D" else "E-effect-hold"
            self.o.require(
                0 <= self.e.elapsed() - self.observation(label, case)["elapsed"] <= 5,
                "fresh positive hold evidence required; no remaining-time inference",
            )
        if case == "E" and not cleanup:
            cut = self.e.state["e_v5"]["pre_cancel_cut"]
            self.c["E_cancel_boundary"] = {
                "cut_sha256": cut["cut_sha256"],
                "hold_elapsed": self.observation("E-effect-hold", "E")["elapsed"],
                "fresh_run_elapsed": self.e.elapsed(),
                "run": run,
                "attempt": 1,
            }
        self.effect(
            "cancel:" + case,
            case,
            PREFIX + "/actions/runs/" + str(run) + "/cancel",
            None,
            202,
            "aborted-own-run-cleanup" if cleanup else "planned-fixture-cancellation",
        )
        if case == "E" and not cleanup:
            self.c["E_cancel_boundary"]["accepted_elapsed"] = self.e.elapsed()
            self.e.persist()


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--custody", required=True)
    commands = parser.add_subparsers(dest="command", required=True)
    preflight = commands.add_parser("preflight")
    preflight.add_argument("--main-ci-run", type=int, required=True)
    for name in ("dispatch", "discover", "diagnose-discovery", "cancel", "cleanup"):
        command = commands.add_parser(name)
        command.add_argument("--case", choices=list("ABCDE"), required=True)
        if name in {"cancel", "cleanup"}:
            command.add_argument("--run", type=int, required=True)
        if name == "cancel":
            command.add_argument("--durable-custody")
        if name == "cleanup":
            command.add_argument(
                "--reason",
                choices=["source", "identity", "budget", "evidence", "deadline"],
                required=True,
            )
    args = parser.parse_args(argv)
    observer = load_observer()
    with observer.locked(args.custody):
        controller = Controller(observer, args.custody)
        controller.e.observation_marker()
        try:
            if args.command == "preflight":
                controller.preflight(args.main_ci_run)
            elif args.command == "dispatch":
                controller.dispatch(args.case)
            elif args.command in {"discover", "diagnose-discovery"}:
                found = controller.discover(args.case, args.command == "diagnose-discovery")
                print(
                    json.dumps(
                        {
                            "outcome": (
                                "DIAGNOSTIC_ONLY"
                                if args.command == "diagnose-discovery"
                                else "BOUND"
                            )
                            if found
                            else "NOT_YET_OBSERVED",
                            "requests": controller.e.state["requests"],
                            **controller.diagnostics(),
                        }
                    )
                )
                controller.e.observation_marker(finish=True)
                return 0
            else:
                controller.cancel(
                    args.case,
                    args.run,
                    args.command == "cleanup",
                    getattr(args, "durable_custody", None),
                    getattr(args, "reason", None),
                )
            controller.e.observation_marker(finish=True)
            print(
                json.dumps(
                    {
                        "outcome": "RECORDED",
                        "requests": controller.e.state["requests"],
                        **controller.diagnostics(),
                    }
                )
            )
        except (Exception, KeyboardInterrupt) as error:
            controller.c["aborted"] = True
            controller.e.persist()
            controller.e.observation_marker(finish=True)
            # No exception message can reveal private URLs, credentials or response bodies.
            print(
                json.dumps(
                    {
                        "outcome": "INCONCLUSIVE",
                        "reason": (
                            "controller refused or effect outcome unavailable; do not retry effects"
                        ),
                        **controller.diagnostics(error),
                    }
                )
            )
            return 2
    return 0
