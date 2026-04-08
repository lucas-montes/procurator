# Gerrit in Repohub

This document explains the **actual Gerrit logic** in `repohub`: change lifecycle, patchsets, votes, readiness, submit flow, and how API/UI share the same core behavior.

## Mental Model

## Change
A **Change** is a review thread for one proposed update to a target branch.

## Patchset
A **Patchset** is a version of that same change.

- Patchset 1 = first upload
- Patchset 2+ = revised versions after feedback

You usually keep adding patchsets to the same change until it is ready to submit.

## Votes / Labels
Reviewers vote on labels (currently `Code-Review` and `Verified`).

## Readiness
A change is submittable only if required checks pass.

Current defaults:
- `Code-Review >= +2`
- `Verified >= +1`
- integration check flag must pass

## State transitions
- `New` → `Merged` on successful submit
- `New` → `Abandoned` on abandon
- `Abandoned` → `New` on restore

## New Change vs New Patchset
Create a **new change** when:
- it is a separate idea/feature/fix and should be reviewed independently.

Upload a **new patchset** when:
- you are revising the same proposed change.

## How Repohub Implements This

## Hexagonal flow
1. UI page or API call hits `adapters/gerrit/web.rs`
2. Web handler invokes shared internal logic (same for UI/API)
3. Shared logic calls application ports/use-cases (`application/gerrit/*`)
4. Ports are implemented by `SqliteReviewRepository` in `adapters/gerrit/persistence.rs`
5. Persistence uses `adapters/shared/database.rs`
6. Final presentation differs only at the end:
   - API => JSON
   - UI => Askama templates

This is why API and UI stay consistent: same DTOs and same core execution path.

## UI routes
- `GET /gerrit/{username}/{project}/{repo}/changes/ui`
- `GET /gerrit/{username}/{project}/{repo}/changes/{change_id}/ui`

## API routes
- `POST /gerrit/{username}/{project}/{repo}/changes`
- `GET /gerrit/{username}/{project}/{repo}/changes`
- `POST /gerrit/{username}/{project}/{repo}/changes/{change_id}/patchsets`
- `POST /gerrit/{username}/{project}/{repo}/changes/{change_id}/votes`
- `GET /gerrit/{username}/{project}/{repo}/changes/{change_id}/submit-readiness`
- `POST /gerrit/{username}/{project}/{repo}/changes/{change_id}/submit`
- `POST /gerrit/{username}/{project}/{repo}/changes/{change_id}/abandon`
- `POST /gerrit/{username}/{project}/{repo}/changes/{change_id}/restore`

## Practical workflow
1. Open repository page and click **Open Gerrit Changes**
2. Create a change (subject, target branch, initial revision)
3. Reviewers vote on labels
4. Upload new patchset if revisions are requested
5. Check readiness
6. Submit when ready (or abandon/restore)

## API walkthrough

Create change:

```json
POST /gerrit/alice/myproj/api/changes
{
  "subject": "Add health endpoint",
  "target_branch": "main",
  "revision": "abc123",
  "kind": "web_upload"
}
```

Upload patchset:

```json
POST /gerrit/alice/myproj/api/changes/42/patchsets
{
  "revision": "def456",
  "uploader_username": "alice",
  "kind": "web_upload"
}
```

Vote:

```json
POST /gerrit/alice/myproj/api/changes/42/votes
{
  "reviewer_username": "bob",
  "label": "Code-Review",
  "value": 2
}
```

Readiness + submit:

```json
GET /gerrit/alice/myproj/api/changes/42/submit-readiness
POST /gerrit/alice/myproj/api/changes/42/submit
```
