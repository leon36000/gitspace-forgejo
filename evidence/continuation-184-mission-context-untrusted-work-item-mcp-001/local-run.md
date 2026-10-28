# CONTINUATION-184 — remettre le contexte d’issue non fiable à l’outil MCP

Date : 27 octobre 2026. Base de travail observée : `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; worktree sale et fortement modifié. Aucun commit créé.

## Problème observé

CONT-183 vérifiait que l’admission produisait `untrusted_work_item` dans `MissionHostAdmissionV1.request`. La revue du consommateur a ensuite montré que `src/bin/mission-012-mcp-host.rs::mission_context` exposait les exigences et le contexte de code mais omettait cette propriété. Le vrai MCP host n’envoyait donc pas ce contexte au client qui appelle l’outil.

## Correction et preuve rouge/verte

`mission_context` transmet maintenant le DTO `untrusted_work_item` sérialisé sous une clé de premier niveau séparée; `requirements_json` et le paquet de code restent dans leurs champs distincts. L’objet conserve route, date de mise à jour, confiance, corps typé et empreinte. La description de l’outil MCP et `trust_boundary` disent explicitement que le texte issue est un contexte utilisateur non fiable et ne peut pas modifier les exigences, permissions, vérification ou autorité de merge. Quand le champ facultatif est absent, l’outil omet cette clé.

Le test `complete_stdio_mcp_lifecycle_runs_red_write_green_shutdown_refusal_and_exit` a d’abord échoué contre l’ancien code : le JSON de `mission_context` donnait `null` au lieu de l’objet fourni par le handoff. Après le changement, le vrai processus `mission-012-mcp-host` passe 1/1. Les assertions comparent l’objet complet retourné au DTO de l’admission, vérifient que le texte hostile reste à part des exigences, et confirment que la description MCP marque le contenu comme non fiable.

Limite de provenance du run rouge : l’empreinte du fichier de test à l’instant rouge n’a pas été sauvegardée; `source-hashes.txt` contient l’empreinte du test final après correction et rustfmt. Le log rouge conserve néanmoins le test ciblé, l’échec exact `left: Null` contre l’objet attendu et le code de sortie 101; ne pas traiter le hash final comme celui du snapshot rouge.

Le test `mcp_host_restarts_from_an_exact_unresolved_workspace_write_intent` passe aussi 1/1 avec un handoff dépourvu de contexte d’issue; il vérifie explicitement que `mission_context` n’émet pas la propriété facultative. Ce run est un contrôle ciblé du comportement de reprise après modification de la construction du contexte.

Logs conservés : `test-red.log`, `test-green.log`, `legacy-context-green.log`; résultats lisibles : `test-summary.txt`.

## Commandes

```sh
cargo test --offline --locked --test mission_012_mcp_lifecycle complete_stdio_mcp_lifecycle_runs_red_write_green_shutdown_refusal_and_exit -- --exact --nocapture
cargo test --offline --locked --test mission_012_mcp_lifecycle mcp_host_restarts_from_an_exact_unresolved_workspace_write_intent -- --exact --nocapture
```

Après les modifications, `cargo fmt --all -- --check`, `git diff --check`, contrôle ciblé des espaces finaux et `python3 scripts/verify_cargo_license_state.py` réussissent. La version Cargo demeure sans licence choisie. Les empreintes des tests, host, scripts, docs et logs sont dans `source-hashes.txt`.

Ce sont des tests de processus locaux avec dépôt temporaire et `SyntheticAdmissionSource`; l’hôte MCP est lancé comme enfant du test sous le même utilisateur. Le test ne se connecte pas à Forgejo et ne revendique pas une frontière Bubblewrap pour l’hôte. Aucun AgentOS, LLM ou autre modèle n’est lancé; aucun prompt n’est évalué. La propriété JSON rend l’information accessible à un client MCP mais ne garantit pas que celui-ci respecte l’avertissement contre l’injection. Le hash atteste seulement la cohérence locale des champs. Ce lot ne fait aucune PR ou fusion, ni benchmark comparatif.

Conclusion : l’interface MCP expose désormais le contexte d’issue courant en gardant la frontière de données explicite. L’intégration agent réel et le comportement du modèle restent non démontrés; le projet demeure incomplet.
