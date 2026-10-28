# MISSION-REQUIREMENT-STALE-PROCESS-REFUSAL-001

Date : 26 octobre 2026. Worktree local examiné à HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, fortement modifié. Aucun commit n’a été créé.

## Question testée

Après acceptation durable d’un webhook d’issue signé HMAC, le document d’exigences protégé peut-il changer sans augmenter sa révision et être tout de même admis par le vrai processus `forgejo-mission-local`? L’admission reçoit un digest SHA-256 épinglé par le host avant l’édition. Le test change le texte et les critères d’acceptation en gardant `REQ-001` et `revision=1).

## Résultat du processus

Le receiver Rust accepte le webhook synthétique en HTTP 202. Après cette acceptation, la fixture HTTP loopback fait lire au processus mission les octets d’exigence modifiés sur la route de contenu fixe. Le CLI quitte avec le code 1 et renvoie `verdict=inconclusive`, `error.code=mission_requirement_hash_mismatch` et une action demandant de revoir la modification protégée et de mettre à jour le digest host avant admission.

Le test relit l’inbox : une seule livraison reste durable. Il rejoue le journal de mission et observe zéro événement d’allocation; le fichier de handoff hôte n’existe pas. Il confirme trois lectures de l’issue sur les routes de fixture attendues et la lecture du fichier d’exigences au commit configuré. Le scénario ciblé passe 1/1, `n=1`.

Le contrôle se trouve dans `src/mission_admission.rs` : après lecture/parsing et vérification de la liaison issue/ID/révision, le code compare le digest host attendu au SHA-256 des octets lus, puis n’appelle `allocate_generation` qu’après cette comparaison. Le test de processus est `tests/forgejo_mission_delivery_process_e2e.rs::actual_mission_cli_refuses_same_revision_requirement_edit_after_signed_delivery`.

## Faux vert découvert dans le runner

Le premier lancement du nouveau test est sorti avec le code 0 et `running 0 tests`, car le runner utilise `--exact --ignored` et le nouveau cas n’avait pas encore l’annotation `#[ignore]`. Ce n’était pas une preuve positive. L’ajout au runner d’une exigence de exactement un test nommé et passant fait désormais échouer cette sortie vide; `guard-red.log` conserve ce refus. Le test a ensuite été correctement marqué pour le runner isolé.

Un premier contrôle de sortie rejetait à tort une forme valide de `--nocapture`, où `ok` apparaissait à la ligne suivante du nom du test. Ce comportement de harness est conservé dans `process-pass-final.log`; le garde final accepte les deux formes et ses fixtures vérifient aussi le rejet de zéro test et du mauvais nom. `process-pass-final-v2.log` est le run positif final : 1 passé, 0 échoué, 0 ignoré.

Le test de processus précédent `actual_mission_cli_admits_signed_webhook_after_closed_refusal_and_reuses_allocation` repasse 1/1 avec l’extension de la fixture, dans `existing-admission-regression.log`.

## Reproduction et empreintes

Les commandes, versions et hachages des inputs sont archivés dans `commands.txt`, `toolchain.txt` et `source-hashes.txt`. Le test a été compilé hors ligne depuis les sources verrouillées sous Bubblewrap avec namespaces réseau et PID séparés. Le répertoire source est monté en lecture seule; le `.runtime` du test est un montage temporaire privé. L’API est une fixture loopback à l’intérieur de ce namespace et le secret HMAC est synthétique. Aucun credential de fusion n’est fourni.

Le manifeste local vérifie les 20 fichiers payload; les dernières empreintes des manifestes globaux du worktree et du projet sont aussi vérifiées pour tous les artefacts de ce lot, le code concerné et le checkpoint. Neuf lignes historiques mal formées dans les manifestes plus anciens sont conservées, sans être traitées comme valides.

Vérifications de clôture de ce lot : `cargo fmt --all -- --check`, `git diff --check`, AST du runner, fixtures du garde, vérification Cargo de licence et ses 9 tests passent. `verification-results.json` enregistre les résultats. L’état Cargo reste `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, droits `not_assessed`, SPDX `NOASSERTION`; aucune licence n’est choisie et aucun droit de redistribution n’est affirmé.

## Portée et limites

C’est une preuve processus du CLI réel contre un serveur HTTP fixture. Elle établit un refus fail-closed avant allocation quand les octets d’exigences divergent du digest host, même si l’identifiant de révision n’a pas bougé. Elle ne démontre pas une livraison émise par Forgejo vivant, un modèle LLM, AgentOS, Forge Code, RAG ou mémoire, la création d’une PR, la vérification indépendante, la fusion ou un comportement post-fusion. Le digest host est une entrée de confiance du test; la manière de protéger son émission en exploitation reste une responsabilité du host et de sa politique.

Aucun service externe, coût de modèle, instance de production, code upstream Forgejo/GitSpace ou donnée privée n’a été utilisé. Aucun changement de licence ou lockfile. Cette preuve ciblée ne démontre aucun gain général ni résultat du benchmark apparié.
