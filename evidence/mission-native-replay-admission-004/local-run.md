# WEBHOOK-MISSION-ADMISSION-NATIVE-REPLAY-001 — run local final (24 octobre 2026)

## Périmètre

Ce run vérifie l’idempotence de l’admission quand Forgejo rejoue nativement une issue vers le receiver et crée un nouvel UUID. Il ne remplace pas le parcours composé CPU-055 déjà consigné dans le checkpoint; il isole la frontière webhook→mission admission.

Commande exécutée depuis le worktree source :

```sh
python3 scripts/run_forgejo_live_webhook_mission_admission.py \
  --forgejo-binary /tmp/forgejo-17.0.0 \
  --state-dir /tmp/forgejo-mission-native-replay-admission-004 \
  --evidence-dir evidence/mission-native-replay-admission-004 \
  --allocation-id mission012-native-replay-admission-004 \
  --native-replay-admission
```

Forgejo v17.0.0+gitea-1.22.0 a écouté uniquement sur `127.0.0.1:3132`; son binaire a le SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`. Le source officiel v17.0.0 examiné ailleurs est au commit `0d791421f85ca13a63b665053d81ea765fbbc571`; la provenance qui relierait ce binaire précis à ce commit n’est pas établie.

Le HEAD de départ est `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Le worktree est très sale et les sources de ce lot ne sont pas commitées. Python 3.12.3, Cargo 1.95.0, rustc 1.95.0 et Bubblewrap 0.9.0 étaient présents.

## Résultat observé

- Forgejo a livré l’événement `issues/opened`; le receiver l’a journalisé à la séquence 1 et a répondu HTTP 202.
- Le POST authentifié à la route native de replay a produit une seconde task pour la même issue. Les deux tasks ont des UUID distincts, HTTP 202 et les mêmes SHA-256 bruts (`35c143ee29d28ad35cb3a50ccb62b6879c7fd50a948961f61a37c8e930cee125`) et sémantiques (`dfd5f45cecf0470547eb8252304b3f63cf2699452a2dab0f5dc18d474ef846c0`). Le hash sémantique du payload complet reçu dans le journal égale celui des tasks Forgejo; il couvre aussi les champs non utilisés pour sélectionner l’issue.
- L’admission initiale a créé la mission `mission-v1-94c26001d4370e63d3637fd5c06ef13f0496424a39a74bf498b7512b7d2b37b5`, allocation `mission012-native-replay-admission-004`, génération 1. Le même delivery rejoué retourne `already_claimed`. Le nouvel UUID natif retourne également `already_claimed`, conserve le même handoff/allocation, et les SHA-256 avant/après du journal mission et du fichier d’admission sont identiques (voir [native-replay-admission.json](native-replay-admission.json)).
- Un mauvais credential est refusé par `mission_source_unauthorized` avant création des fichiers de claim. Le hook est relu inactif; les deux processus s’arrêtent et aucun listener attendu ne subsiste.
- Le mode sans `--native-replay-admission` a aussi passé un run frais, [mission-admission-default-005](../mission-admission-default-005/local-run.md), qui conserve une seule livraison et n’exécute pas le scénario natif.

## Vérifications et provenance

- 21 tests `test_forgejo_live_webhook_mission_admission` et 9 tests `test_forgejo_webhook_native_replay_e2e` passent (30 total); 5 tests du garde Cargo passent. Sorties : [tests Python](python-tests.log), [garde licence](cargo-license-tests.log), [py_compile](py-compile.log).
- `cargo metadata --locked --offline --format-version 1 --no-deps` et le garde dédié confirment `license=null`, `license_file=null`, `publish=[]`, décision `unselected`, droits de redistribution `not_assessed` : [métadonnées](cargo-metadata.json), [readback](cargo-license-verify.json). Aucune licence n’a été choisie.
- SHA-256 du runner mission `e809fdb156da54822c9ebb031c4b3d80d68f886ef0adde97ccf7f32ce97c68ec`, du helper replay `eda16692077bf97332668c92e27e1f3ab6a11c3a092cd8c3db4ceb3c4c14284a`, des tests mission `0e4e31eb8202e3e50c87b6603ec21ba88e4bba19e2d92c981d741a447ff551a4` et des tests replay `5a81fe51ecc0248407a62f684405866718edf1328c10a106bb780dcfff60f9b9`.
- Le détail est dans [result.json](result.json), [native-replay-binding.json](native-replay-binding.json), [readback des tasks](result.json) et [provenance.json](provenance.json). Les sommes de contrôle locales couvrent les artefacts du run; le manifeste global les référence aussi.

## Limites

L’agent n’a pas été invoqué (`agent_runtime_invoked=false`); aucun changement de code, PR, vérification indépendante de candidat ou merge n’a eu lieu dans cette exécution. Aucun LLM, AgentOS, Forge Code, RAG ou moteur de mémoire n’a été appelé. Il s’agit d’un run `n=1` contre Forgejo local. La lecture SQLite des tasks est spécifique au schéma interne v17, les services partagent l’UID hôte et Bubblewrap n’isole que le build. L’état privé reste sous `/tmp/forgejo-mission-native-replay-admission-004`.
