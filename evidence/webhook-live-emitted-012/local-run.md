# WEBHOOK-LIVE-012 — readback strict et événements Forgejo réels (24 octobre 2026)

## Reproduction

Run parti du HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0` dans le worktree modifié du lot webhook. Les hashes du runner, des tests, des dépendances et des exécutables sont consignés dans [`provenance.json`](provenance.json). Forgejo annonce `17.0.0+gitea-1.22.0` (release `17.0.0`), SHA-256 binaire `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`. L’origine exacte du build du binaire n’est pas établie.

```sh
python3 -m unittest discover -s tests -p 'test_forgejo_emitted_webhook_e2e.py' -v
python3 -m py_compile scripts/run_forgejo_live_webhook_reconcile_emitted.py tests/test_forgejo_emitted_webhook_e2e.py
python3 scripts/verify_cargo_license_state.py
python3 scripts/run_forgejo_live_webhook_reconcile_emitted.py \
  --forgejo-binary /tmp/forgejo-17.0.0 \
  --repo-root "$PWD" \
  --state-dir /tmp/forgejo-live-webhook-emitted-012-20261024-01 \
  --evidence-dir "$PWD/evidence/webhook-live-emitted-012"
(cd evidence/webhook-live-emitted-012 && sha256sum -c SHA256SUMS)
```

Le build Rust utilise Bubblewrap 0.9.0 avec réseau et namespace PID désactivés, environnement nettoyé, sources/registre/sysroot en lecture seule, Cargo `--offline --locked`. Rejouer avec de nouveaux chemins : le runner refuse de remplacer son état ou ses preuves existants.

## Résultat observé

Le GET Forgejo après le PATCH de désactivation confirme le même ID de hook `1`, type `gitea`, content-type `json`, cible loopback effective, `active=false` et `configuration_matches_expected=true`. Les sélecteurs `issues` et `pull_request` sont relus sous forme de 15 événements effectifs; `events_readback_valid=true`. Aucun secret n’est dans le reçu. Les quatre événements émis réellement (issue ouverte/fermée, PR ouverte/fermée) correspondent aux tâches Forgejo et à leurs UUID; chacune est livrée avec succès en HTTP 202. L’issue #1 et la PR #2 sont fermées; la PR n’est pas fusionnée, tête `89df26a623c33f9a97bcd697fc2cfd2e00b6387a`, base `4edccefb5b3aecc3627ffa4a7caf77cecf03dc82`. Les anciens et nouveaux événements se réconcilient à l’état courant lu par API. Le credential invalide reste inconclusif.

Forgejo et le receiver sortent avec code 0; les ports observés sont libres. L’état privé `/tmp/forgejo-live-webhook-emitted-012-20261024-01` reste en mode `0700`, incluant DB/config/secret de test. Les preuves sont aussi dans un répertoire `0700`, fichiers `0600`, et les sommes de [`SHA256SUMS`](SHA256SUMS) passent.

## Revue indépendante et refus

La revue read-only de WEBHOOK-LIVE-011 a confirmé le binding au GET réel, l’omission du secret, le PATCH, les quatre tâches et les manifestes. Elle a relevé qu’une valeur non textuelle dans `events` pouvait être filtrée avant la comparaison et ainsi permettre un faux match. Le test reproduit ce cas en rouge; le sérialiseur conserve seulement les chaînes pour l’affichage, expose `events_readback_valid=false` et refuse `configuration_matches_expected`. L’ensemble ciblé des suites webhook, admission et composition compte 78 tests et passe après ce correctif. Les manifestes des essais négatifs 009/010 et du présent run passent.

Le code officiel Forgejo tagué [`v17.0.0`](https://codeberg.org/forgejo/forgejo/src/tag/v17.0.0/routers/api/v1/utils/hook.go) développe les événements regroupés; son `editHook` applique `push` si les sélecteurs sont absents. Le runner transmet donc explicitement les sélecteurs sélectionnés pendant la désactivation.

## Limites

Cette tranche démontre l’émission réelle, la réception, le readback strict et la réconciliation; la PR n’est ni vérifiée ni fusionnée dans ce run. Les services partagent l’UID hôte et aucun code candidat hostile n’est exécuté. Le build exact du binaire Forgejo n’est pas prouvé. Aucun LLM, AgentOS, Forge Code, RAG ni mémoire n’est invoqué et aucun gain de valeur n’est mesuré. Le parcours complet suivant utilise explicitement `local-test-adapter`; il ne sera pas présenté comme une exécution LLM.

Les états secrets 008–012 sont laissés intacts conformément à la demande de confirmer avant toute suppression.
