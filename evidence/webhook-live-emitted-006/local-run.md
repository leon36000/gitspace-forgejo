# WEBHOOK-LIVE-006 — webhook Forgejo émis et état courant réconcilié

Date : 24 octobre 2026 (EDT). Worktree observé à HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, avec d’autres changements suivis et non suivis préexistants. La provenance lie les hashes des sources Rust et des quatre scripts, du registre Cargo mis en scène, de Bubblewrap, du toolchain et des binaires. Le fichier de test Python n’était pas inclus dans cette provenance; cela a été corrigé au run 007. Le binaire Forgejo v17 local est identifié par SHA-256; son lien de build exact avec le clone source officiel au commit `0d791421f85ca13a63b665053d81ea765fbbc571` n’est pas établi.

## Reproduction

```sh
python3 -m unittest discover -s tests -p 'test_forgejo_emitted_webhook_e2e.py' -v
python3 -m py_compile scripts/run_forgejo_live_webhook_reconcile_emitted.py tests/test_forgejo_emitted_webhook_e2e.py
python3 scripts/verify_cargo_license_state.py
git diff --check
python3 scripts/run_forgejo_live_webhook_reconcile_emitted.py \
  --forgejo-binary /tmp/forgejo-17.0.0 \
  --repo-root "$PWD" \
  --state-dir /tmp/forgejo-live-webhook-emitted-006-20261024-02 \
  --evidence-dir "$PWD/evidence/webhook-live-emitted-006"
```

Les cinq tests unitaires passent. Ils couvrent la validation du dépôt et de l’issue, le rejet du booléen Python comme numéro, l’appariement UUID/événement et l’ajout privé de l’allowlist à `app.ini`. `py_compile`, le garde de métadonnées Cargo et `git diff --check` passent; leurs sorties sont conservées dans `emitted-006-*.log`. Ce dernier contrôle couvre le diff suivi; il ne vérifie pas seul les fichiers non suivis.

Le driver reconstruit `forgejo-webhook-local` et `forgejo-reconcile-local` sous Bubblewrap avec namespace réseau/PID, HOME masqué, environnement nettoyé, sources/registre/sysroot en lecture seule, Cargo offline/locked et limites de ressources. La configuration Forgejo de test autorise seulement les callbacks vers `127.0.0.1`. Les preuves sont dans [`provenance.json`](provenance.json) et [`cargo-build.log`](cargo-build.log).

## Résultat observé

- Forgejo `17.0.0+gitea-1.22.0` crée une issue privée #1 et émet deux webhooks `issues` réels vers le receiver Rust : `opened` puis `closed`. Les deux tâches `hook_task` sont marquées livrées/réussies avec HTTP 202; leurs UUID correspondent exactement, en ordre, aux `delivery_id` du journal.
- Le CLI de réconciliation traite séparément l’ancien événement `opened` et le nouvel événement `closed`, et relit pour chacun l’API Forgejo actuelle : verdict `current_state_read`, une lecture, état `closed`. La notification ancienne ne devient pas la source de vérité.
- `forge-reader` est non admin et en lecture seule sur le dépôt (`admin=false`, `push=false`, `pull=true`). Un credential invalide produit exit 1, `inconclusive/upstream_unauthorized`, sans `canonical_state`.
- Le readback confirme que le hook est désactivé. Les deux processus suivis terminent avec code 0 et les listeners attendus sur Forgejo `127.0.0.1:3130` et le receiver n’existent plus. Cela vérifie ces PID et ces ports pour ce run, pas l’absence universelle de descendants ou d’autres ressources.
- [`SHA256SUMS`](SHA256SUMS) couvre le rapport, les readbacks, les résultats, les journaux de vérification et la provenance. Le dossier d’évidence est en mode `0700`, ses fichiers en `0600`.

## Portée et limites

Ce run compose réellement l’émission Forgejo, la réception HMAC, le journal Rust et la relecture d’état issue. Il ne réalise pas la demande→exigence→mission→modification→PR→vérification→fusion. Le readback des tâches s’appuie sur le schéma SQLite interne `hook_task` de Forgejo v17, utile comme observation de ce harnais mais non garanti comme API produit. L’instance Forgejo, le receiver et le CLI s’exécutent sous le même UID hôte; aucun code candidat hostile n’est lancé. Les doublons/désordres et les reprises après crash ne sont pas exercés par ce run précis.

Aucun LLM, AgentOS, RAG ou service mémoire n’a été utilisé; ce n’est pas une intégration d’agent ni une mesure de valeur, de coût ou de temps. La bibliothèque de test allowlist loopback est limitée à l’instance jetable et ne convient pas à une configuration de production.

Un premier lancement a échoué avant le démarrage d’un service car le helper de configuration utilisait une création exclusive pour mettre à jour un `app.ini` existant. Le correctif d’append sécurisé a été couvert par le test unitaire; aucun hook ni processus n’a été créé lors de cet essai. Les deux répertoires `/tmp/forgejo-live-webhook-emitted-006-20261024-01` et `...-02` sont conservés en mode privé; le second contient la base et le secret temporaire du run, exclu des preuves persistées et non supprimé.

## Note de revue après exécution

La revue read-only a trouvé que [`hook-readback.json`](hook-readback.json) enregistrait `active_after_run=true`, alors que le GET Forgejo confirmé et [`result.json`](result.json) établissent `false`. Il s’agit d’une inversion de sérialisation de `hook_disabled`; elle ne démontre pas que le hook soit resté actif. Le résultat général est utile, mais ne pas utiliser ce champ du run 006 comme preuve d’état. WEBHOOK-LIVE-007 corrige le champ avec un test rouge/vert, ajoute la source de test à la provenance et relance le run complet : voir [son rapport](../webhook-live-emitted-007/local-run.md). La provenance 006 ne hache pas le fichier de test Python; 007 le fait.
