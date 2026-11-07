# Fraud investigation demo

A small synthetic retail-banking dataset: customers, their accounts, the
devices they log in from, card transactions and merchants, plus a second
customer source (KYC records) with deliberate near-duplicates for entity
resolution. Every person, account, device and merchant is fictional.

```bash
make demo-fraud
```

The loader uses only Core's public REST API, verifies the result and prints
the Graph ID and an Explorer link. Running it again reuses and checks the
existing graph `Demo: Fraud investigation`; nothing is duplicated or deleted.
Without make: `docker compose --profile demo run --rm demo fraud`. Natively:
`cargo run -p mindgraph-cli -- demo fraud --api http://127.0.0.1:8080`.

## The data

| File | Rows | Becomes |
|---|---|---|
| `customers.csv` | 5 | `Customer` (C001 to C005) |
| `accounts.csv` | 7 | `Account`, and `Customer -OWNS-> Account` |
| `devices.csv` | 4 | `Device`, and `Customer -USES-> Device` (with `first_seen`) |
| `transactions.csv` | 8 | `Transaction` and `Merchant`, `Account -SENT-> Transaction -TO-> Merchant` |
| `customers_kyc.csv` | 8 | more `Customer` records (K-1001 to K-1008) with email and phone |

`ontology.json` defines the five entity types and four relationship types;
each CSV has a `*.mapping.json` (the M4 mapping format) and `demo.json` lists
the load order and the checks below.

## What the loader checks

| Check | Expected |
|---|---|
| Entities | 13 Customer, 7 Account, 3 Device, 8 Transaction, 4 Merchant |
| Relationships created | 27 (7 OWNS, 4 USES, 8 SENT, 8 TO) |
| Alice Tan (C001) | owns 2 accounts |
| DEVICE-921 | used by 2 customers |
| CryptoSwap Exchange (M004) | receives 3 transactions |
| Path C001 → C002 | 2 hops: `USES · USES` (through DEVICE-921) |
| Path C001 → M004 | 3 hops: `OWNS · SENT · TO` |
| Entity resolution | at least 7 candidate pairs from email, phone and fuzzy-name rules |

## What to look at

1. **A shared device.** In the Explorer, Alice Tan opens with her
   neighbourhood. Expand DEVICE-921: Budi Santoso uses it too. Find Path from
   Alice to Budi returns the 2-hop path.
2. **Payments to the same merchants.** Expand Alice's account A1001 and
   Budi's account A1003: both paid QuickPay Electronics (899.00, seven
   minutes apart) and CryptoSwap Exchange (six minutes apart).
3. **Possible duplicate customers.** Open Entity Resolution: the KYC file
   repeats some customers with different spelling, letter case and phone
   formats ("ALICE  TAN", "Budi Santosa", "M. Ikhwan Nasution"). Review a
   candidate and link two records as the same entity, or keep them separate.
   Linking never merges or deletes graph data.

These are patterns worth reviewing, not findings: nothing in the data is
labelled as fraud, and a shared device or merchant can have innocent
explanations.

## Ask MindGraph (optional)

With an LLM provider configured, ask on Alice Tan: *"Which customers share
a device with Alice Tan?"* The answer cites the entities and relationships it
used. Without a provider everything above still works.
