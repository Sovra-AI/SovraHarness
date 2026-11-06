# Stage 4 — Regulated work (Direction)

> **Direction, not today.** Health, law, finance, government: data that must not
> leave. Here, it never has to.

An outline, researched 2026-11-05. Regulations move, so dates below must be
re-checked before any sales or compliance claim. Items marked *(verify)* came
from secondary sources. This document is **not legal advice**. Engage
counsel per sector before selling.

## 1. The core message, stated carefully

Local-first removes whole categories of obligation:

- cross-border transfer paperwork
- vendor processor or business-associate status (often)
- a central breach target

It does **not** remove the obligations that regulated customers carry:

- access control and MFA
- audit logs
- encryption at rest
- retention schedules and legal hold
- eDiscovery and data-subject access/erasure
- model risk documentation
- human oversight
- staff training
- the customer's own risk assessments

There is no vendor cloud to do these things, so **they must ship inside the
product**.

## 2. Sector requirements at a glance

| Sector | Key rules | What local-first buys | What Sovra must still build |
| --- | --- | --- | --- |
| **Health (US)** | HIPAA Security Rule: risk analysis, access control, **audit controls** §164.312(b), integrity, transmission security (<https://www.hhs.gov/hipaa/for-professionals/security/index.html>). The proposed 2025 update (encryption required, MFA, asset inventory, 72-hour restore) is **not final**; target about July 2027 (<https://www.federalregister.gov/documents/2025/01/06/2024-30983/hipaa-security-rule-to-strengthen-the-cybersecurity-of-electronic-protected-health-information>). | If Sovra-the-company never receives PHI (no telemetry, no relay of content, local models), a vendor BAA may be unnecessary. | Per-user audit, unique ids, auto-logoff, encryption at rest by default, backups. Cloud LLMs only via BAA-covered providers. Build to the proposed rule now. |
| **EU / UK privacy** | GDPR Art. 5 (principles), 25 (by design), 32 (security), 35 (DPIA), 17 (erasure), 44–49 (transfers; Schrems II) (<https://gdpr-info.eu/>). UK GDPR plus the Data (Use and Access) Act 2025. | No international transfer when processing stays on EU devices. The vendor may not be a processor at all. | "Find and export everything about a person" across memory, erasure (purge payload, keep hash), retention, records of processing, DPIA templates. A cloud model call is a transfer, so add a per-lane switch. |
| **EU AI Act** | In force 1 Aug 2024. The Digital Omnibus moved Annex III high-risk duties to **2 Dec 2027** and Annex I to **2 Aug 2028**. Art. 50 transparency has applied since 2 Aug 2026 (<https://fpf.org/blog/the-ai-act-implementation-timeline-what-changes-under-the-ai-omnibus/>). | Sovra is a general-purpose tool, not a GPAI model provider. Customer LoRA is probably far below the "substantial modification" threshold *(verify)*. | Deployer logging, human-oversight controls, "you are talking to AI" disclosures, FRIA support for public bodies using Annex III cases. |
| **Law** | ABA Formal Opinion 512 (29 Jul 2024): competence, confidentiality, supervision, fees. **Self-learning tools need informed client consent** (<https://www.americanbar.org/content/dam/aba/administrative/professional_responsibility/ethics-opinions/aba-formal-opinion-512.pdf>). UK Law Society guidance is similar in spirit. | A local, non-self-learning model with no vendor access is the cleanest position. | **One lane per matter or client**, ethical walls (no cross-matter retrieval), **company model off by default** in legal lanes, legal hold (suspend purge), eDiscovery export. |
| **Finance** | SEC 17a-4: WORM, **or** a complete time-stamped audit trail (2022 amendment) (<https://www.mayerbrown.com/en/insights/publications/2022/10/sec-adopts-amendments-to-electronic-recordkeeping-requirements-for-broker-dealers-and-security-based-swap-entities>). FINRA 2210/3110. The 2026 FINRA report adds supervision of **AI agents** (<https://www.finra.org/rules-guidance/guidance/reports/2026-finra-annual-regulatory-oversight-report/gen-ai>). SR 11-7 was replaced by **SR 26-2** (17 Apr 2026), which excludes generative AI pending an RFI. DORA (EU, since 17 Jan 2025). SOX ITGCs. | Supersede-never-delete memory is naturally close to the 17a-4 audit-trail alternative. | Retention locks per lane, non-rewriteable export to the customer's WORM store, supervision queues for AI-drafted client communications, agent action logs, model inventory and lineage for MRM teams, DORA contract annex and exit plan. |
| **Government** | FedRAMP covers *cloud* services, so a desktop app is outside it, but any Sovra-hosted relay is inside. CJIS v6.0 (FIPS-validated crypto, MFA, audit). UK Cyber Essentials (Plus). | No hosted service means no FedRAMP scope for the core product. | **FIPS 140-3** crypto option (aws-lc-rs FIPS with rustls, <https://github.com/aws/aws-lc-rs>), air-gapped edition, hardening guides. |
| **Sovereignty laws** | India DPDP (Rules Nov 2025; most duties around May 2027), China PIPL (export assessments) *(verify thresholds)*, Saudi PDPL, UAE PDPL *(verify)*. | Data stays in-country by default. | Residency attestations generated from the ledger (stage 3, A3). |

## 3. Product controls to build (the regulated baseline)

These are additions to stages 1–3, not a separate product:

1. **Encryption at rest by default** for the memory store and lane payloads,
   with keys in the OS keychain or TPM.
2. **SSO and MFA** (from stage 3, A0), plus auto-logoff.
3. **Per-user audit log**: who read, wrote, exported or shared what, and
   which helper and model were involved. Exportable (OCSF) and verifiable
   against ledger checkpoints.
4. **Retention and legal hold** per lane. A hold suspends purge, and
   retention locks enforce minimum keep periods.
5. **Data-subject requests**: find everything about a person (facts,
   episodes, artifacts, files index), export it, and erase it (purge
   payloads, keep hashes, log the erasure).
6. **Cloud kill switch per lane** ("this lane never leaves our computers"),
   enforced in routing (`routing_tree.rs`) and in work handoff.
7. **No-telemetry mode** and an **offline licence**: a signed licence file
   verified locally, with a grace period.
8. **Supervision queues**: AI-drafted outbound communications wait for a named
   reviewer (FINRA 3110). The approval-card pattern already exists.
9. **Agent action log**: every tool call, with arguments, approver and
   outcome. Today's terminal and approval cards are the start of this.
10. **Matter and ethical-wall lanes** for law: no retrieval across matter
    lanes, and no company-model training on them without recorded consent.

## 4. Supply chain and air-gapped operation

- **Updates and models via TUF** (`tough` crate), mirrorable to an internal
  server or USB (<https://theupdateframework.io/>).
- **Signed model bundles** (OpenSSF Model Signing / Sigstore,
  <https://github.com/sigstore/model-transparency>) with offline verification.
- **SBOM** per release (CycloneDX via `cargo-cyclonedx`), **SLSA Build L3**
  provenance (<https://slsa.dev/>), **reproducible builds**, and
  `cargo-auditable`. `cargo-deny` already runs in CI.
- US federal sales need the **Secure Software Development Attestation**
  (NIST SSDF, <https://www.cisa.gov/secure-software-attestation-form>).
- **Confidential computing** for optional shared inference servers only:
  AMD SEV-SNP, Intel TDX, or NVIDIA confidential mode with attestation.
  Model it on Apple's Private Cloud Compute
  (<https://security.apple.com/blog/private-cloud-compute/>): computers send
  prompts only to servers whose attested image hash is on the company ledger.

## 5. Certifications (in order)

1. **SOC 2 Type II**, covering Sovra's own build, signing, update and support
   operations.
2. **ISO/IEC 27001:2022**.
3. **ISO/IEC 42001:2023** (AI management system), which maps to the AI Act
   and NIST AI RMF (<https://www.iso.org/standard/81230.html>).
4. As needed: HITRUST (US health), Cyber Essentials Plus (UK government),
   FIPS 140-3 module use, Common Criteria / NIAP (defence).

A local product has a small audit scope. The SDLC and supply chain *become*
the scope, which keeps these cheaper.

## 6. Phases

- [ ] **B0, controls baseline.** Encryption at rest, SSO/MFA, audit log,
  retention and hold, data-subject requests, cloud kill switch per lane,
  offline licence, no-telemetry mode, and a HIPAA / GDPR / Op. 512
  configuration guide. Start SOC 2. *Target buyers:* small clinics and law
  firms.
- [ ] **B1, records and supervision.** 17a-4 audit-trail mode with retention
  locks and WORM export, supervision queues, agent action logs, eDiscovery
  export, matter lanes and ethical walls.
- [ ] **B2, air-gapped edition.** TUF mirrors, signed models, SBOM, SLSA,
  reproducible builds, FIPS option, CJIS hardening guide. ISO 27001, then
  ISO 42001.
- [ ] **B3, AI governance pack.** Model inventory and lineage from the ledger,
  evaluation-gate reports, pre-filled DPIA/FRIA templates, Art. 50
  disclosures, deployer logging ahead of Dec 2027, DORA annex and exit plan.
- [ ] **B4, sovereign deployments.** Residency attestations per country,
  attested shared inference, partner-led accreditations.

## 7. Risks

- Regulated buyers buy **evidence**: certifications and documentation are
  12–24 months of non-feature work.
- The company model conflicts with legal ethics (Op. 512, ethical walls) and
  with GDPR purpose limitation, so it stays off by default in regulated lanes.
- Rules move: the HIPAA final rule (around 2027), AI Act dates already shifted
  once, and US bank model-risk guidance has a gap for generative AI.
- Hospitals and government run shared workstations and VDI (Citrix, AVD)
  without GPUs. Desktop-local AI may need a site inference node.
- FIPS and air-gapped builds fork the release process.

## 8. Open questions

- Which vertical first: small law firms and clinics (fast; local-first
  resonates) or banks (slow; large)?
- Will Sovra offer a BAA for any optional relay?
- Does the 17a-4 audit-trail option need third-party attestation of the
  ledger?
- What is the product copy that reconciles "never deleted" with "erased"
  (payload purged, hash kept)?
- Does customer fine-tuning trigger AI Act GPAI-modifier duties? This needs an
  opinion from EU counsel.
- Is VDI support required? Which FIPS module, and does it fit the rustls-only
  rule?
