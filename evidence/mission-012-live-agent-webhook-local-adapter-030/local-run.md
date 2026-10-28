# MISSION-012-COMPOSED-006 — webhook réel jusqu’à la fusion Rust locale (24 octobre 2026)

## Reproduction et identité

Depuis le worktree `.runtime/webhook-005-workspace`, HEAD initial examiné `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0` (worktree déjà modifié; aucune affirmation de checkout propre) :

```sh
python3 scripts/run_mission_012_live_composed_e2e.py \
  --forgejo-binary /tmp/forgejo-17.0.0 \
  --state-dir /tmp/mission012-composed-adapter-030 \
  --evidence-dir "$PWD/evidence/mission-012-live-agent-webhook-local-adapter-030" \
  --execution-id live-mcp-030 \
  --allocation-id mission012-local-adapter-030 \
  --budget-cap-usd 0.20 \
  --agent-adapter local-test-adapter
```

Forgejo annonce `17.0.0+gitea-1.22.0`, release `17.0.0`, SHA-256 du binaire `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`. Le build exact de ce binaire depuis le tag officiel n’est pas établi. Les scripts composés, l’identité de l’adaptateur, les reçus et les hashes figurent dans `composed-run.json` et `composed-stage-summaries.json`; une comparaison des hashes de sources enregistrés au run avec les fichiers présents est conservée dans `source-hash-checks.json` (toutes les entrées correspondent; le worktree initial était sale).

## Résultat observé

Forgejo a émis `issues/opened` pour l’issue #1. L’UUID du delivery Forgejo correspond à la tâche du receiver et du journal; la réponse est HTTP 202. Le claim d’admission lie l’allocation `mission012-local-adapter-030`, la base `0c73ed61547d692bb592c0a55dcaa9a9bde277b3` et l’exigence `MISSION-012-CLAMP-001` révision 1 (SHA-256 `3a3c444075c93a3bcfb2bec17d1a2e74ec0051effc2994e6b02ac095eb0dd431`). Le rejeu renvoie `already_claimed` sans modifier les fichiers d’admission/journal. Un credential hôte invalide est refusé avant création d’admission ou de journal. Le GET Forgejo après désactivation confirme le hook réellement lu, inactif, avec les cinq événements effectifs attendus et sans secret sérialisé.

L’adaptateur a consommé `mission.context`, `workspace.read_text`, `workspace.write_text` et `cargo.test`, avec huit reçus. Il a d’abord exécuté l’acceptance sur la base inchangée : échec attendu du cas de borne supérieure (`-2` au lieu de `10`; exit 101). Il a ensuite modifié uniquement `src/lib.rs`. Le commit candidat `330bb0db4262262a8ea002e931c7faac6f6cfe0d` (SHA-256 du fichier candidat `173304ad7d32aabc1e8e1ee41eabda3398a8690798e68f906bcebc73cbcd6d15`) passe les deux tests d’acceptance. Les critères versionnés et les sorties base rouge/candidat vert sont conservés dans les preuves.

L’acceptance indépendante `local-isolated-cargo-acceptance-v1` reconstruit le candidat exact depuis l’API Forgejo dans un clone hôte de confiance et le teste sous Bubblewrap avec namespaces mount/PID/réseau, snapshot en lecture seule, cible Cargo fraîche, sans HOME ni credentials hôte et avec Cargo offline/locked. Une seconde vérification avant fusion relit le même candidat, la même exigence et la même empreinte de policy et retourne `pass`. La PR #2 a été refusée avant revue par Forgejo (HTTP 405, « Does not have enough approvals »); la tentative de fusion du reviewer non autorisé est aussi refusée HTTP 405. Après l’approbation de fixture, l’autorité locale `forgejo-reconcile-local Rust` utilise le credential owner via un pipe hérité privé, fusionne avec HTTP 200 et relit le commit `a783f44a6bb8fa962ef75aefd023e4a15e59ce31`. Ses parents sont exactement la base et le candidat; le hash d’arbre relu correspond à l’arbre candidat; l’issue #1 et la PR #2 sont fermées. Ce n’est pas une autorité de production.

## Vérifications conservées

Une revue read-only déléguée a comparé les claims du rapport, les reçus/manifeste et les hashes de sources; elle n’a trouvé aucune contradiction concrète. Sa couverture et ses limites sont consignées dans `independent-review.md`.

Le lot de contrats Python comprend 78 tests dans les suites webhook émis, admission webhook, composition, MCP agent, PR agent et admission E2E; ils passent. `py_compile` des six runners et de leurs tests passe. Le build de l’autorité Rust `cargo build --offline --locked` passe. Les essais Cargo base rouge et candidat vert, la vérification indépendante, la re-vérification prémerge, les refus Forgejo, les readbacks et les sommes SHA-256 sont sous ce dossier et dans `../mission-012-live-agent-webhook-local-adapter-030-admission/`. Le rapport, les journaux et reçus de ce dossier ont des permissions `0600`; le dossier est `0700`. Les états Forgejo locaux privés et logs d’exécution sous `/tmp` sont conservés en `0700`/`0600`.

Le garde de licence rend `license=null`, `license_decision=unselected`, `publish=[]` et `redistribution_rights=not_assessed`; cela ne tranche aucun droit de réutilisation.

## Limites

Ce parcours utilise explicitement `local-test-adapter`, déterministe et écrit pour cette petite fixture. Aucun modèle n’a été appelé dans ce run; les compteurs de tokens sont nuls/non disponibles, le coût modèle déclaré est zéro, et aucune comparaison appariée avec un agent LLM et une CI conventionnelle n’a été faite. Ce n=1 ne mesure pas une amélioration générale. AgentOS, Forge Code, RAG et mémoire n’ont pas été invoqués.

La fixture Forgejo ne versionne pas `policy/active.json` ni la policy v4. Le vérifieur exécute les critères Cargo protégés sur le commit exact, mais l’empreinte de policy vient du fixture hôte; elle n’est donc pas démontrée comme exigence de policy versionnée avec le dépôt. Le verrou est coopératif et ne protège pas contre un écrivain externe/non coopératif; Forgejo ne fournit pas ici de CAS atomique sur la base au POST de fusion. Les processus de service partagent l’UID local. L’isolation Bubblewrap observée concerne le vérificateur, pas une séparation de privilèges de production.

Suite utile : éprouver SIGKILL et reprise au seuil POST/readback, puis un changement concurrent sémantique et un writer de base non coopératif; intégrer la policy versionnée dans le même binding; établir une référence appariée multi-tâches avant de conclure sur l’efficacité. Les nouvelles sessions Claude restent soumises au budget explicitement autorisé et au risque que `--max-budget-usd` soit un plafond estimatif CLI, pas une limite de facture.
