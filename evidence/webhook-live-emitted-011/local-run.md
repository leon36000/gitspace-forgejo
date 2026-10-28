# WEBHOOK-LIVE-011 — lecture réelle de la configuration du hook (24 octobre 2026)

## Reproduction

Le run est parti de HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, dans un worktree déjà très modifié. Les empreintes des deux sources Python effectivement utilisées sont dans [`provenance.json`](provenance.json). Forgejo annonce `17.0.0+gitea-1.22.0`, release name `17.0.0`; le binaire local a le SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`. L’origine exacte du build du binaire n’est pas établie.

```sh
python3 -m unittest discover -s tests -p 'test_forgejo_emitted_webhook_e2e.py' -v
python3 -m py_compile scripts/run_forgejo_live_webhook_reconcile_emitted.py tests/test_forgejo_emitted_webhook_e2e.py
python3 scripts/verify_cargo_license_state.py
python3 scripts/run_forgejo_live_webhook_reconcile_emitted.py \
  --forgejo-binary /tmp/forgejo-17.0.0 \
  --repo-root "$PWD" \
  --state-dir /tmp/forgejo-live-webhook-emitted-011-20261024-01 \
  --evidence-dir "$PWD/evidence/webhook-live-emitted-011"
(cd evidence/webhook-live-emitted-011 && sha256sum -c SHA256SUMS)
```

Le build Rust s’est fait avec Bubblewrap 0.9.0, réseau et namespace PID désactivés, environnement nettoyé, sources/registre/sysroot en lecture seule, Cargo `--offline --locked`; détails et hashes sont dans [`provenance.json`](provenance.json) et [`cargo-build.log`](cargo-build.log). Pour rejouer le scénario, choisir de nouveaux chemins d’état et de preuves : le runner refuse de réutiliser ceux de ce reçu.

## Résultat

Le readback Forgejo après désactivation confirme l’identifiant de hook `1`, le type `gitea`, le mode `json`, l’URL loopback effective, `active=false`, et `configuration_matches_expected=true`. L’API développe les deux sélecteurs `issues` et `pull_request` en 15 événements effectifs. Le reçu [`hook-readback.json`](hook-readback.json) reprend la réponse réelle, sans enregistrer le secret.

Le receiver Rust a reçu quatre événements émis par les tâches webhook Forgejo : issue #1 ouverte puis fermée, PR #2 ouverte puis fermée. Les quatre tâches sont livrées avec succès en HTTP 202; leur UUID correspond aux identifiants du journal. La PR porte la tête `86e90d3cc5fdfce733b9d15033a967a548f5d6aa` sur la base `102619212fb09628961d53917e18d71b0f220be7`, puis reste fermée et non fusionnée. Les notifications anciennes et nouvelles se réconcilient toutes à l’état API actuel `closed`; l’issue est également fermée. Ces constats sont liés dans [`result.json`](result.json), les quatre readbacks de réconciliation et [`forgejo-hook-tasks.json`](forgejo-hook-tasks.json).

Le credential invalide produit `inconclusive/upstream_unauthorized`, sans état canonique. Le hook est inactif avant l’arrêt; Forgejo et le receiver sortent avec code 0, et les ports contrôlés ne restent pas en écoute. Le répertoire privé `/tmp/forgejo-live-webhook-emitted-011-20261024-01` est conservé en mode `0700`; il contient la base, la configuration et le secret de test. Aucun de ces secrets n’est dans les reçus. Le répertoire de preuves est en mode `0700`, ses fichiers en `0600`, et son manifeste est vérifié.

## Corrections révélées par les runs 009 et 010

La revue de WEBHOOK-LIVE-008 a découvert que son reçu de configuration sérialisait les valeurs attendues au lieu de les lire dans la réponse API. Le run 009 a fait échouer le contrôle nouvellement ajouté : après le `PATCH` de désactivation, Forgejo renvoyait réellement `events=["push"]`. Le run 010 a conservé les événements dans le corps du `PATCH`, mais a révélé que la réponse API développe les sélecteurs `issues` et `pull_request` en leurs 15 événements effectifs. Ces reçus d’échec sont conservés dans les dossiers 009 et 010 avec leurs manifestes valides.

Le code Forgejo officiel tagué [`v17.0.0`](https://codeberg.org/forgejo/forgejo/src/tag/v17.0.0/routers/api/v1/utils/hook.go), fonction `editHook`, remet les événements à `push` lorsque le champ `events` est vide; la fonction `addHook` développe les sélecteurs groupes vers les sous-événements. Le runner transmet donc explicitement les deux sélecteurs au PATCH de désactivation et compare le readback réel à l’ensemble effectif. Le run 011 est le premier reçu de cette série qui confirme la configuration complète après désactivation. Il ne modifie pas rétroactivement les artefacts du run 008.

## Vérifications et limites

Onze tests Python ciblés passent; `py_compile`, le garde de licence et le contrôle ciblé des espaces finaux passent. Le garde retourne `verified`, mais la licence Cargo reste `null`, la décision de licence `unselected` et les droits de redistribution `not_assessed`. Les résultats exacts se trouvent dans les journaux de ce dossier.

Ce run démontre le comportement local Forgejo→receiver→réconciliation d’événements issue et PR, mais ne vérifie pas le code candidat, ne le fusionne pas et ne constitue pas à lui seul le parcours produit complet. Forgejo, receiver et CLI partagent l’UID de l’hôte; aucun code candidat hostile n’est exécuté. Aucun LLM, AgentOS, Forge Code, RAG ou service mémoire n’a participé; aucun gain de réussite, temps, interventions humaines, tokens ou coût n’est mesuré. La démonstration d’un agent LLM reste suspendue à une autorisation de dépense avec plafond réellement applicable; l’essai Claude précédent a dépassé son plafond estimatif et Ralph n’a pas continué.

Les répertoires secrets des runs 008 à 011 sont laissés intacts. Leur nettoyage nécessite une décision explicite, comme demandé pour les données privées temporaires.
