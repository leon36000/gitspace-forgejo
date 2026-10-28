# CONTINUATION-299 — adaptateur HTTP d’arbitrage v3 et politique hôte

Date : 27 octobre 2026. Worktree local fortement modifié, base examinée `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; aucun commit, déploiement, publication, reset ou nouvel accès. Le lot ne copie aucun code Forgejo ou GitSpace.

## Changements exécutables

- `src/bin/forgejo-mission-local.rs::LocalLoopbackForgejoSource` implémente `MissionArbitrationSourceV1`. Il lit la PR dans le dépôt et le profil demandés, puis récupère les avis à `GET /pulls/{n}/reviews?page=P&limit=50`. `X-Total-Count` est extrait du JSON d’en-têtes de curl; l’absence, les valeurs multiples, un nombre non numérique, une variation entre pages, un compte incohérent, une réponse surdimensionnée ou une pagination incomplète échouent fermés. Limites locales : 10 000 avis, 1 MiB de corps par page et 16 MiB au total; l’agrégat est aussi borné par le contrat commun.
- Le CLI lit le document d’exigences depuis la tête protégée courante. Le schema 3 sélectionne seulement `.runtime/mission-arbitration/<profil-hôte-fixe>.json`; aucun argument CLI ne peut choisir un chemin ou des octets. Le chargeur vérifie l’UID effectif, les modes `0700` des répertoires et `0600` du fichier, l’absence de lien supplémentaire/symlink final, la taille, le JSON strict et le scope exact du profil. Le gate v3 journalise sa décision et n’alloue pas en cas de politique ou de preuves invalides.
- Tests du binaire couvrent deux vraies pages HTTP loopback (51 avis), le lien PR/scope, les refus d’en-tête manquant/dupliqué, de count dérivant, de pagination trop grande, de dépôt étranger, l’absence d’accès réseau lors d’un scope refusé, le credential gardé sur stdin, l’emplacement fixe et les permissions de la politique. Les IDs de revue utilisés sont des valeurs synthétiques de fixture.

Le code amont Forgejo v17.0.0 a été vérifié dans le scratch d’audit au commit `0d791421f85ca13a63b665053d81ea765fbbc571`. `routers/api/v1/repo/pull_review.go::ListPullReviews` documente `page` et `limit` (lignes 50–57), appelle `FindReviews`/ `CountReviews`, puis `SetTotalCountHeader(count)` avant le JSON (lignes 84–108). Empreinte SHA-256 du fichier : `0ada741ac5530bced27b3764dccec77826a3f9ac2a706dfcc1dc1d824dd23d52`. Le fichier ciblé n’a pas de diff au commit; le scratch complet reste modifié par d’autres POC. Aucun code n’est repris.

## Vérifications locales

`cargo test --locked --offline --all-targets` : **715 réussis, 0 échec, 155 ignorés**, 38 blocs de résultat de suite. Les tests ignorés, notamment certains parcours qui exigent un runner Bubblewrap/Forgejo, ne sont pas comptés comme exécutés. Le run ciblé du lot : **101 réussis, 0 échec** (CLI 43, arbitrage v3 12, admission v2 19, webhook 15, runtime 12).

`cargo check --locked --offline --all-targets`, `cargo fmt --all -- --check`, `git diff --check` et `scripts/verify_cargo_license_state.py` réussissent. Versions : Rust/Cargo 1.95.0, curl 8.5.0, Bubblewrap 0.9.0, Linux aarch64. Les commandes et sorties sont dans ce dossier; les hashes sont dans `source-sha256.txt` et `upstream-source.txt`.

Le test reproductible `test_host_policy_hidden_from_candidate_bwrap.py` crée uniquement une politique synthétique sous un checkout temporaire et un répertoire candidat vide. L’hôte observe le fichier présent; dans une commande Bubblewrap au montage candidat-seul dérivée du sandbox MISSION-012, `test ! -e <chemin-hôte>` réussit. Ce constat précis ne couvre pas un autre processus hors sandbox sous le même UID, ni le noyau/root, ni tous les chemins de l’hôte.

Les probes API sur `127.0.0.1:3130` et `:3132` ont refusé la connexion. Pas de PR/revues Forgejo live, pas de succès CLI v3 complet, pas d’ID de réviseur réel connu ni de fichier de politique provisionné. Le CLI v3 réel reste donc fail-closed en l’absence de politique; aucune identité de fixture n’est présentée comme réelle. L’API Forgejo n’identifie pas les comptes comme humains ou bots. La pagination d’avis n’est pas un snapshot atomique; les relectures du gate détectent certaines dérives sans supprimer la course après la dernière lecture. L’instance loopback et son adaptateur restent sandbox-only.

La garde de licence rapporte toujours `license=null`, `publish=[]`, `no_license_selected`, droits de redistribution non évalués et `NOASSERTION`; ce contrôle de métadonnées n’est pas une décision juridique.

## État de complétion et suite

Ce lot relie le lecteur HTTP réel du produit au contrat v3 et ajoute le chargement fixe hôte; il ne démontre pas un parcours réel d’arbitrage sur Forgejo. La preuve Bubblewrap est une sonde de visibilité de fichier, pas une certification d’isolation. Le parcours LLM/AgentOS, les tests cumulatifs manquants, la comparaison contrôlée, l’identité humaine et les licences restent à établir.

Prochaine tranche locale : assembler une fixture HTTP Forgejo pour lancer le binaire compilé sur le chemin v3 avec une politique temporaire de scope fixe et mesurer les créations d’allocation/refus au niveau du processus. Ce résultat resterait une fixture. Un test live ultérieur dépend d’une instance locale démarrée et d’une allowlist d’identifiants de comptes effectivement autorisés; aucun ID ne sera deviné ni demandé à un service externe.
