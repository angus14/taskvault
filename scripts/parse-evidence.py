#!/usr/bin/env python3
"""Build the demo timeline from passing sandbox transactions and account views."""
import json
import pathlib
import sys

log = pathlib.Path(sys.argv[1]).read_text()
lines = [line for line in log.splitlines() if line.startswith("TASKVAULT_EVIDENCE ")]
if len(lines) != 1 or "test result: ok. 1 passed; 0 failed" not in log:
    raise SystemExit("Expected one successful sandbox evidence record")
fields = dict(part.split("=", 1) for part in lines[0].split()[1:])
required = {
    "vault_funded": "1000", "budget": "100", "chosen": "first",
    "first_paid": "70", "second_70_rejected": "true",
    "duplicate_rejected": "true", "alternative_blocked": "true",
    "failed_transfer_released": "true", "fallback_paid": "25",
    "spent": "95", "unused_withdrawn": "5",
}
if fields != required:
    raise SystemExit(f"Unexpected sandbox evidence: {fields!r}")
if log.count("TASKVAULT_RACE submitted=2 accepted=1 rejected=1 spent=70 budget=100") != 1:
    raise SystemExit("Expected one passing concurrent-submission result")
fields["concurrent_requests"] = "2"
fields["concurrent_accepted"] = "1"
events = [json.loads(line.split(" ", 1)[1]) for line in log.splitlines()
          if line.startswith("TASKVAULT_EVENT ")]
agents = [json.loads(line.split(" ", 1)[1]) for line in log.splitlines()
          if line.startswith("TASKVAULT_AGENT ")]
steps = ["approved", "first_paid", "over_budget", "duplicate", "alternative",
         "handoff", "handoff_retry", "transfer_failed", "fallback_paid",
         "unused_returned", "race"]
if [event["step"] for event in events] != steps:
    raise SystemExit("Sandbox timeline is incomplete or out of order")
for event in events:
    if not isinstance(event["tx"], str) or len(event["tx"]) < 30:
        raise SystemExit("Sandbox event has no transaction hash")
    total = event["spent"] + event["reserved"] + event["available"]
    if total != (event["spent"] if event["closed"] else event["budget"]) or total > event["budget"]:
        raise SystemExit(f"Inconsistent budget snapshot: {event['step']}")
if len({event["tx"] for event in events}) != len(events):
    raise SystemExit("Two demo events reused a transaction")
if [(a["account"], a["offer"], a["amount_milli_near"], a["status"], a["tx"])
    for a in agents] != [
    ("agent-a.sandbox", "first", 70, "settled", events[1]["tx"]),
    ("agent-c.sandbox", "fallback", 25, "settled", events[8]["tx"])]:
    raise SystemExit("Independent agent transactions do not match sandbox views")
if [(e["spent"], e["vendor_delta"]) for e in events if e["step"] in
    ("approved", "first_paid", "fallback_paid", "unused_returned", "race")] != [
    (0, 0), (70, 70), (95, 95), (95, 95), (70, 70)]:
    raise SystemExit("Vendor or spending evidence differs from the expected workflow")
trace = {"schema": 1, "units": "milliNEAR", "summary": fields,
         "agents": agents, "events": events}
out = pathlib.Path(sys.argv[2])
out.write_text("window.taskvaultTrace = " + json.dumps(trace, sort_keys=True) + ";\n")
json_path = out.with_suffix(".json")
json_path.write_text(json.dumps(trace, indent=2, sort_keys=True) + "\n")
print(f"Generated {out} and {json_path} from passing sandbox transactions")
