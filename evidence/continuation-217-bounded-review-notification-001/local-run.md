# CONTINUATION-217 — borne du contexte de notification de revue

Date : 27 octobre 2026. Worktree `.runtime/webhook-005-workspace`, HEAD examiné `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, avec de nombreux changements locaux conservés. Ce lot modifie uniquement la projection de la note de revue, les versions de contrats publiques et leurs consommateurs/tests; aucun fichier Forgejo amont n’est copié ou modifié.

## Changement

`review_notification.body` garde au plus 8 192 octets UTF-8. Si la limite tombe au milieu d’un caractère, la fin du préfixe recule jusqu’à une frontière valide. La réponse conserve `body_truncated`, la longueur complète en octets et le SHA-256 du corps source avant troncature. Un corps absent ou `null` n’obtient pas de longueur ni de digest. Le hash identifie seulement le contenu observé; il ne prouve ni authenticité, ni correction, ni approbation.

La note reste sous `review_notification`, distincte de `canonical_state`, étiquetée `untrusted_user_content` et `event_snapshot_not_canonical`. La notification n’est pas une preuve ou autorité de fusion. La liaison à un ID de revue demeure indisponible dans le payload Forgejo v17. La projection est événementielle; l’état canonique provient toujours du readback existant.

La sortie de réconciliation incrémente ses schémas de single v9 à v10 et batch v10 à v11. La découverte des capacités incrémente son contrat v5 à v6 et publie `review_notification_projection` avec source, champs, limite, sémantique du hash, confiance et fraîcheur. Les consommateurs Rust/Python et fixtures concernés sont migrés ensemble.

## Méthode et résultats

Le test `oversized_review_notification_is_utf8_bounded_with_source_fingerprint` a été ajouté avant le changement de code. Son run rouge observait le corps source complet au lieu du préfixe attendu. Après implémentation, il passe et couvre 8 191 octets ASCII suivis d’un emoji quatre octets : le préfixe finit à la frontière Unicode précédente, tandis que longueur et digest représentent le corps complet.

Le test CLI `actual_cli_discloses_versioned_local_test_capabilities` vérifie v6, single v10, batch v11 et le document de projection. Les trois suites Python touchées passent : emitted webhook 131/131, native replay 30/30, reviewer readback 29/29. Les cinq pilotes Python modifiés passent `py_compile`.

La commande `cargo test --locked --offline --all-targets -- --test-threads=1` termine avec code 0. Le journal complet et son résumé donnent 669 réussis, 0 échec, 124 ignorés, 36 résumés de cibles. Les tests ignorés indiquent leurs runners Bubblewrap/ports dédiés dans le journal; ce rapport ne prétend pas qu’ils ont été exercés par ce run. `cargo fmt --all -- --check` et `git diff --check` passent. `cargo metadata --locked --offline --no-deps --format-version 1` rapporte `agent-forge-core 0.1.0`, `license=null`, `license_file=null`, `publish=[]`; le vérificateur de politique rapporte `no_license_selected`, droits `not_assessed`, SPDX `NOASSERTION`.

Commandes et sorties conservées dans ce dossier : `commands.txt`, `rust-all-targets.log`, `rust-all-targets-summary.json`, trois journaux Python, `pycompile.log`, `cargo-license-state.log`, `cargo-metadata.json`, versions des outils et hashes source. L’environnement est Rust/Cargo 1.95.0 et Python 3.12.3.

## Portée et réserves

Ce lot démontre des contrats locaux, pas une nouvelle exécution sur Forgejo live. Il n’appelle pas de LLM/AgentOS, ne crée pas de PR/merge et ne mesure pas l’efficacité du contexte. Le plafond de corps réduit l’amplification par livraison, mais le budget agrégé de sortie reste une protection distincte. Le SHA-256 n’est ni une signature ni une garantie de validité. La version publique change; les clients qui inspectent les schémas doivent migrer. Aucun choix de licence, coût, accès ou service externe n’a été engagé.

Le projet reste incomplet : AgentOS/LLM et son isolation réellement autorisés, benchmark comparatif apparié, droits/licences complets, autorité de fusion atomique contre tous les writers et plusieurs scénarios de refus cumulatifs restent ouverts.
