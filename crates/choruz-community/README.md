# Choruz community

Validate, exchange and aggregate behavior evidence without running the Choruz platform. Local and public records share one schema. Private trace references and context belong to the caller's storage, not extra fields on an exchange record.

`behavior::BehaviorRecord` validates bounded records and solution versions. `behavior::counts` deduplicates independent occurrences and preserves later disputes; its output counts contributor claims, not model failure rates. Missing observed model identity stays absent rather than borrowing the configured model.

`hub::Hub` reads immutable revisions of the public dataset named by `behavior::COMMUNITY_REPOSITORY`, reuses unchanged cached objects and submits contributions as review PRs. Fetching does not send local queries. The caller owns cache persistence, consent, privacy review and durable publication state. Schema validation cannot detect all sensitive prose. Do not publish a record until those checks pass, or automatically retry an uncertain dispatched contribution.

## Use without the platform

Add this directory as a Cargo path dependency. The library depends on `choruz-common` and `choruz-evaluation`, not platform storage, agent processes or the API server. All are in the same workspace and version line; using a library does not require deploying the product.

Run the deterministic example to serialize evidence, restore it and aggregate a disputed success without a database, model call or public upload:

```sh
cargo run -p choruz-community --example evidence
```

Package the libraries together without publishing them:

```sh
cargo package -p choruz-common -p choruz-evaluation -p choruz-community --allow-dirty
```

The existing platform community worker composes this library with its authorized database, privacy review and outbox. Library results do not authorize installing guidance or activating an Agent revision.
