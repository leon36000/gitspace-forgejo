# CONTINUATION-181 — revue du flux de consommation

Date : 27 octobre 2026. Cette revue suit l’ajout de `work_item_content`; elle vérifie où le champ peut effectivement arriver, sans lancer de modèle.

## Parcours des données constaté

- `ReconciledState.work_item_content` est produit par `src/webhook/reconcile.rs` et sérialisé par le CLI réconciliateur dans `canonical_state`. Le runner de webhook et les tests valident la source, la confiance et les bornes.
- La recherche littérale du nom `work_item_content` dans `src`, `tests` et `scripts` ne trouve pas de consommation dans `src/mission_admission.rs` ni `src/agent_runtime.rs`. Son usage produit est limité au reconciler; les autres occurrences sont les tests et le runner de readback.
- `IssueSnapshotV1` contient seulement `number`, `html_url` et `state` (`src/mission_admission.rs:47-52`). `parse_issue_snapshot` lit seulement ces trois propriétés (`src/bin/forgejo-mission-local.rs:538-569`). La réponse API d’issue peut porter titre/corps, mais ces données ne passent pas cette frontière de type.
- `build_context_packet` prend le document d’exigences protégé, son chemin et commit, puis les octets du fichier source; il construit le texte à partir du `request` et des critères d’acceptation de ce document ainsi que de l’extrait de code épinglé (`src/bin/forgejo-mission-local.rs:936-1000`). Il n’a aucun argument issue title/body.
- `admit_delivery` transmet le paquet créé à `admit_reconciled_issue_v2`, puis ne ressort du `ReconciledState` que l’identité de livraison, l’état et `updated_at` (`src/bin/forgejo-mission-local.rs:1817-1855`). `AgentExecutionRequestV1` lie les exigences par hash au commit du workspace; aucune donnée `work_item_content` n’y est ajoutée.

## Vérification ajoutée

Le test `issue_snapshot_requires_the_canonical_url_and_supported_state` utilise une issue dont le titre et le corps demandent de remplacer les critères, puis change ces deux chaînes. Les deux réponses produisent le même `IssueSnapshotV1`. Le ciblage passe 1/1; la sortie est `rust-mission-issue-content-boundary.log`. `cargo fmt --all -- --check` et `git diff --check` passent après l’ajout (`rustfmt-post-audit.log`, `diff-check-post-audit.log`). Les empreintes des sources parcourues sont dans `consumer-audit-source-hashes.txt`.

## Conclusion et limites

La projection issue/PR est disponible aux consommateurs de réconciliation comme contexte courant non fiable, mais l’admission de mission ne la fournit pas au runtime. Cela protège les exigences dans ce parcours et signifie aussi que le runtime ne peut pas s’appuyer sur l’issue comme contexte additionnel. Aucun LLM/AgentOS n’est invoqué; le parcours et le test sont locaux. La revue par recherche source et signature de fonctions n’exclut pas un consommateur hors de `src`, `tests` et `scripts`, ni une modification ultérieure du câblage. Si un futur lot transmet ces champs à l’agent, il doit les garder séparés du document d’exigences, cités au commit/à la source, marqués non fiables et bornés; cette nouvelle voie exigera un test end-to-end de non-substitution des critères.
