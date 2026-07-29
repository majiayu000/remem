# SpecRail Workflow Retirement

On 2026-07-29, PR #965 removed the repository-vendored SpecRail execution pack
from Remem. The retired surface included repo-local skills, workflow/state/label
configuration, checks, schemas, templates, sync locks, sensitive-governance and
closure workflows, and their dedicated CI tests.

Remem now relies on its native contributor guidance, product CI, normal review,
and explicit human merge and release authorization. The `docs/specs/` contracts
and root-level `specs/GH*/` issue packets remain in the repository as product,
technical, and historical implementation evidence; they do not activate a
workflow.

The retired executable files remain recoverable through Git history before the
final PR #965 migration commit. No copy is kept in the current tree because
retaining an offline pack would continue to expose discoverable skills and
state-machine assets.
