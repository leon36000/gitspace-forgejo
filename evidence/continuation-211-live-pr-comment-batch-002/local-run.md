# CONTINUATION-211 — batch live PR et commentaire d’organisation

## Résultat

Une instance jetable Forgejo `17.0.0+gitea-1.22.0` a créé une organisation privée, une équipe `reviewers`, un dépôt privé, une PR ouverte et non fusionnée, puis un webhook loopback. Forgejo a émis au récepteur Rust une delivery `pull_request/review_requested` suivie d’une delivery native `issue_comment/created` pour la conversation de cette PR. L’API Forgejo a confirmé l’équipe courante `{id:2,name:reviewers}`.

Le CLI `forgejo-reconcile-local` a reçu les deux IDs dans cet ordre avec le token read-only (`read:user,read:repository,read:issue`). Il a retourné le schéma batch v9 et `current_state_read` pour les deux événements, liés à la même PR, tête et base courantes. L’access log Forgejo montre exactement deux lectures HTTP réussies pendant ce batch : `GET /pulls/1` et `GET /issues/comments/3`, toutes deux `200`. Le commentaire lu par son ID a le corps créé par le harnais et `body_trust=untrusted_user_content`; aucune action de mission ou décision de fusion n’en découle. Le journal webhook est identique octet pour octet avant et après la réconciliation.

Le contrôle distinct préexistant du runner a aussi tenté `POST /issues/1/comments` avec le token read-only : Forgejo a répondu `403`, sans changement des commentaires ni du fingerprint PR. L’instance et le récepteur sont arrêtés avec code 0 et aucun listener ne reste sur 3130 ou le port loopback du récepteur.

## Reproduction

Commande réussie :

```sh
python3 scripts/run_forgejo_live_org_team_reviewer_readback.py \
  --forgejo-binary /tmp/forgejo-17.0.0 \
  --state-dir /tmp/forgejo-team-review-cont211-state-002 \
  --evidence-dir evidence/continuation-211-live-pr-comment-batch-002 \
  --probe-pr-comment-batch
```

Le rapport exécute le vrai serveur Forgejo local, son webhook, le processus receiver Rust et le vrai CLI Rust. La compilation des binaires se fait dans Bubblewrap 0.9.0 avec `--unshare-net --unshare-pid`, source et sysroot montés en lecture seule et Cargo `--offline --locked`. L’instance Forgejo et le receiver utilisent les adresses loopback du harnais. Cela ne constitue pas une politique générale d’egress pour un agent.

Environnement : Python 3.12.3, Rust/Cargo 1.95.0, Forgejo `17.0.0+gitea-1.22.0`, SHA-256 binaire `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`. Le commit exact de build Forgejo est inconnu. Le reçu `build-provenance.json` identifie les exécutables Rust construits et le hash de l’arbre source staged; le reçu `result.json` contient les IDs et états synthétiques nécessaires sans credential.

## Développement et contrôles

Les tests du nouveau normaliseur d’IDs batch et des validateurs de résultat / access log ont d’abord été lancés avant leur implémentation et ont échoué par import attendu. Après correction, le module Python ciblé passe `22/22`; `py_compile` réussit pour les deux runners et le module de test. Les mutations du validateur refusent notamment l’ordre inversé, un mauvais parent/tête/base, une équipe absente, un commentaire marqué fiable, une lecture API absente/dupliquée/étrangère ou un statut différent de 200. Le garde Cargo et ses tests passent `12/12`; le reçu courant confirme `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, droits `not_assessed` et `NOASSERTION`.

Un premier essai Forgejo, conservé dans `failed-harness-attempt-001.log`, a créé les deux webhooks puis échoué avant la réconciliation parce que le runner appelait un nom de fonction inexistant. Ce défaut de harness a été corrigé et le résultat ne compte pas comme un échec Forgejo. Le run corrigé est `n=1`; le dossier d’état privé initial et le dossier du run échoué sont conservés. Aucun fichier de credential n’est dans le reçu.

## Portée et limites

Cette preuve établit un chemin local intégré Forgejo → webhook receiver → sélection batch du CLI Rust → lectures API courantes, ainsi que l’étiquette de confiance du commentaire. Elle ne prouve pas un agent LLM, AgentOS ou Forge Code, un succès de livraison logicielle, une décision d’acceptation indépendante, une fusion contrôlée, une correction après échec ou un gain comparatif. Le contenu et l’instance sont synthétiques, un seul run est observé, et l’access log provient du serveur testé. Aucun code Forgejo/GitSpace n’est copié. Aucun coût, appel externe, publication ou changement production.

La licence Cargo reste non choisie (`license=null`, `license_file=null`, `publish=[]`); les droits de redistribution restent `not_assessed`. Les critères cumulatifs de la mission restent ouverts, en particulier le parcours agent avec LLM isolé, la comparaison contrôlée et l’autorité de fusion contre tous les writers.
