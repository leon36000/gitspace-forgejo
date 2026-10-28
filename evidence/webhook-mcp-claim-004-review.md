# WEBHOOK-MCP-CLAIM-004 — revues indépendantes read-only

Deux revues de sécurité read-only ont été déléguées à un agent Sonnet 5.5 (`security-reviewer`). L’agent n’a ni modifié de fichier, ni lancé de compilation, ni contacté de service réseau.

## Revue 1 — état initial claim-bound (avant correctifs)

Cibles : `src/bin/mission-012-mcp-host.rs`, `tests/mission_012_mcp_lifecycle.rs`, avec `src/agent_runtime.rs` et `src/mission_012_host.rs` comme contexte.

Verdict : **0 finding HIGH/MEDIUM**. Propriétés confirmées : profil épinglé, aller-retour exact de la requête et liaison durable claim/lease, expiration et verrou contre une horloge reculée, génération périmée, reçus des outils refusés ou inconnus, `create_new` + `O_NOFOLLOW` et synchronisation du fichier et du répertoire, finalisation unique, limites de taille.

Findings LOW relevés, tous traités dans ce lot :

1. Hôte candidat construit après l’acquisition du lease : une mauvaise configuration consommait l’ID de lease jusqu’au TTL.
2. Une erreur persistante de lecture stdin bouclait indéfiniment.
3. Le refus de budget était écrit en best-effort et ignorait les erreurs d’écriture.
4. Les reçus étaient écrits par petits `write()` et le fichier n’était pas ouvert en append.
5. Les chemins du journal et des reçus n’étaient pas comparés aux répertoires du sandbox.

Point INFO corrigé : une assertion unitaire `tools.get("workspace.diff")` ne pouvait jamais échouer.

## Revue 2 — après correctifs

Verdict : **0 finding HIGH/MEDIUM**. Les six correctifs sont confirmés corrects. `CandidateToolHost::new` est sans effet de bord. Aucun contournement de `refuse_state_paths_inside_workspace` n’a été trouvé par simple contrôle de la configuration (`..`, symlinks de parent ou finaux, chemin terminé par `..`, parent absent, préfixe non composant, hard links côté journal). Aucune régression sur les propriétés de la revue 1.

Suites données :

- LOW, test manquant pour une trame trop grande → ajout de `mcp_host_exits_and_records_an_abort_on_an_oversized_frame`.
- LOW, assertion `lease_acquisitions` sans portée dans les cas `journal_file` → laissée telle quelle. Les cas restent discriminants par le code d’erreur sur stderr, par `target/` vide et par la base inchangée.
- INFO, documentation → l’en-tête du module précise maintenant que le lease n’est jamais relâché et que les reçus suivent les effets, et retire « all failing closed » pour la limite même UID.
- INFO non traités, consignés comme limites : un bind mount (root requis) n’est pas détecté ; une trame tronquée en EOF est traitée comme un EOF normal ; `read_locked` de la bibliothèque tronque une queue non JSON d’un journal désigné par la configuration ; une finalisation occupée n’a pas de retry borné.

## Revue séparée de composition WEBHOOK → MCP

Une revue Codex read-only des points d’intégration a examiné `src/bin/forgejo-mission-local.rs`, `src/bin/mission-012-mcp-host.rs`, `src/mission_012_host.rs` et `tests/mission_012_mcp_lifecycle.rs`. Elle ne modifie pas le verdict borné des deux revues du host : elle constate une lacune entre tranches.

- **Composition bloquante pour un parcours joint :** admission actuelle sur `127.0.0.1:3130`, `agent-forge-core`, issue 41 / `REQ-001`; host MCP épinglé à `127.0.0.1:3132`, `mission-012-clamp`, issue 1 / `MISSION-012-CLAMP-001`. Le test construit directement un claim synthétique, sans webhook, CLI d’admission ni réconciliation Forgejo.
- **SHA du workspace non lié :** l’hôte conserve `request.workspace.base_commit` mais ne compare pas `base_dir` à ce commit; l’utilitaire `verify_pair` existe dans `mission_012_host.rs` mais n’est pas utilisé à cette frontière.
- **Fenêtre effet/reçu :** les outils ont un effet avant l’append synchronisé du reçu; un arrêt dans cette fenêtre laisse un candidat changé sans reçu. Le test de sink défaillant n’exerce pas un vrai effet workspace.
- **Provenance du journal :** les fichiers privés et synchronisés protègent certains accidents, pas la modification par un processus du même UID; aucune MAC/signature du journal n’est présente.

Cette revue n’a exécuté ni test ni service. Les revues Sonnet portaient sur le comportement du host local et concluaient 0 finding HIGH/MEDIUM dans leur cible; elles ne couvrent pas ces coutures d’intégration ni l’authenticité du workspace.
