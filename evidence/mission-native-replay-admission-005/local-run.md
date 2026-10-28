# WEBHOOK-MISSION-ADMISSION-NATIVE-REPLAY-001 — run final vérifié (24 octobre 2026)

## Périmètre

Ce run vérifie l’idempotence de l’admission quand Forgejo rejoue nativement une issue vers le receiver et crée un nouvel UUID. Il isole la frontière webhook→mission admission; il ne remplace pas le parcours composé CPU-055 déjà consigné dans le checkpoint.

Commande exécutée depuis le worktree source :

```sh
python3 scripts/run_forgejo_live_webhook_mission_admission.py \
  --forgejo-binary /tmp/forgejo-17.0.0 \
  --state-dir /tmp/forgejo-mission-native-replay-admission-005 \
  --evidence-dir evidence/mission-native-replay-admission-005 \
  --allocation-id mission012-native-replay-admission-005 \
  --native-replay-admission
```

Forgejo v17.0.0+gitea-1.22.0 a écouté uniquement sur `127.0.0.1:3132`; son binaire a le SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`. Le source officiel v17.0.0 examiné ailleurs est au commit `0d791421f85ca13a63b665053d81ea765fbbc571`; la provenance reliant ce binaire précis à ce commit n’est pas établie.

HEAD de départ `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; worktree très sale, sources non commitées. Python 3.12.3, Cargo 1.95.0, rustc 1.95.0 et Bubblewrap 0.9.0.

## Résultat observé

- La livraison `issues/opened` initiale a été reçue à la séquence 1 avec HTTP 202. Le POST authentifié à la route native de replay crée une seconde task pour la même issue; les deux UUID sont distincts et les deux tasks finissent en HTTP 202.
- Le SHA-256 brut des deux tasks Forgejo est `bb8735f68124f611754ffde33ee00d827439c0d2f9b5b0a99e854d79d032a17a`; leur SHA sémantique et celui de l’objet complet journalisé sont identiques : `cfdc72c6d41863f32347427b95e281ca1f2ae972e1ba3850d8620fa671d4af6f`. Le test de contrat construit deux payloads reçus qui ne diffèrent que par un champ secondaire et confirme le refus du binding divergent. La comparaison côté receiver porte sur l’objet JSON canonique; elle n’établit pas l’égalité octet pour octet du corps HTTP avec `payload_content` Forgejo.
- L’admission initiale a créé la mission `mission-v1-94c26001d4370e63d3637fd5c06ef13f0496424a39a74bf498b7512b7d2b37b5`, allocation `mission012-native-replay-admission-005`, génération 1. Rejouer le même UUID puis l’UUID natif neuf retourne `already_claimed`; le nouvel événement conserve le même handoff/allocation et les SHA avant/après du journal mission (`3b53bd88eb00c4dbaaaec54126890ebfe1fa528d511205c7117d9d9a76c9976f`) et du fichier d’admission (`cca622228e6dffbeb8c8c7f5bd706f2dc4445a4855027fb9186f17c00da6578c`) concordent.
- Un mauvais credential est refusé par `mission_source_unauthorized` avant création des fichiers de claim. Le hook est relu inactif, les processus s’arrêtent et aucun listener attendu ne subsiste.
- Un run de compatibilité frais sans `--native-replay-admission` est dans [mission-admission-default-006](../mission-admission-default-006/local-run.md).

## Vérifications et provenance

- 22 tests du runner d’admission et 9 tests du runner de replay passent (31 au total); 5 tests du garde Cargo passent. Voir [tests Python](python-tests.log), [garde Cargo](cargo-license-tests.log) et [py_compile](py-compile.log).
- Le test du champ secondaire divergent a été observé rouge quand la comparaison reçu↔task est retirée, puis vert après rétablissement : [rouge](payload-binding-red.log), [vert](payload-binding-green.log).
- `cargo metadata --locked --offline --format-version 1 --no-deps` et le garde dédié confirment `license=null`, `license_file=null`, `publish=[]`, décision `unselected`, droits de redistribution `not_assessed` : [métadonnées](cargo-metadata.json), [readback](cargo-license-verify.json). Aucune licence choisie.
- SHA-256 du runner mission `e809fdb156da54822c9ebb031c4b3d80d68f886ef0adde97ccf7f32ce97c68ec`, du helper replay `eda16692077bf97332668c92e27e1f3ab6a11c3a092cd8c3db4ceb3c4c14284a`, des tests mission `e8d54c2b7da8be14225fcc9cec62ef3904458f1f40abc151137b090856b44297` et des tests replay `5a81fe51ecc0248407a62f684405866718edf1328c10a106bb780dcfff60f9b9`.
- [result.json](result.json), [native-replay-binding.json](native-replay-binding.json), [native-replay-admission.json](native-replay-admission.json), [provenance.json](provenance.json) et les manifests fixent les reçus du run. Le mot de passe admin et le secret du hook n’y sont pas enregistrés.
- La revue finale indépendante est archivée dans [independent-review.md](independent-review.md); elle confirme le binding sémantique et les manifestes, sans exécuter de commande.

## Limites

L’agent n’a pas été invoqué (`agent_runtime_invoked=false`); aucun changement de code, PR, vérification indépendante de candidat ou merge n’a eu lieu dans cette exécution. Aucun LLM, AgentOS, Forge Code, RAG ou moteur de mémoire n’a été appelé. Run `n=1` contre Forgejo local; lecture SQLite des tasks spécifique à v17; services sous le même UID; Bubblewrap isole seulement le build. L’égalité du corps HTTP brut reçu avec les octets stockés dans Forgejo reste non démontrée; seul le JSON canonique reçu est comparé sémantiquement. L’état privé reste sous `/tmp/forgejo-mission-native-replay-admission-005`.
