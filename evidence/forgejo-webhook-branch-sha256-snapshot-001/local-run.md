# FORGEJO-WEBHOOK-BRANCH-SHA256-SNAPSHOT-001

## Résultat

Le contrat de branche accepte explicitement un commit courant non nul composé de 64 chiffres hexadécimaux. Le test branch_snapshot_accepts_nonzero_sha256_commit_id couvre cette forme; le test existant push_notification_reconciles_current_branch_head_instead_of_stale_after_sha couvre le chemin SHA-1 de 40 caractères. L’ensemble webhook_reconcile_contract passe 53/53.

Le test processus actual_reconcile_cli_process_reads_sha256_current_branch_commit traverse le vrai binaire receiver puis le vrai forgejo-reconcile-local compilé hors réseau dans Bubblewrap. Le receiver accepte HTTP 202 pour un push de création signé avec before nul. Le CLI lit ensuite une unique réponse HTTP 200 de fixture loopback contenant le nom de branche exact et un commit.id synthétique de 64 caractères non nuls. Il rend verdict=current_state_read, state=present, conserve le SHA exact, n’émet aucun Authorization et laisse les octets du journal intacts. Le test isolé passe 1/1, avec 108 cas filtrés. Il complète le refus déjà conservé d’un commit.id nul de 40/64 caractères dans [FORGEJO-WEBHOOK-BRANCH-ZERO-CURRENT-SHA-001](../forgejo-webhook-branch-zero-current-sha-001/local-run.md), qui n’a pas été rejoué ici.

## Cause du premier échec de parcours

La première invocation E2E a compilé le snapshot mais le wrapper s’est arrêté avant lancement du test : son appel à run_forgejo_mission_delivery_process_e2e.run omettait l’argument keyword obligatoire test_name. Le traceback exact reste dans process-e2e.log. La signature du helper et les appels voisins confirment la cause. Le wrapper transmet maintenant test_name=test_name; la même commande a alors lancé le test isolé et passé 1/1. Une première vérification rustfmt avait aussi relevé la mise en forme d’un assert dans le nouveau test; la ligne a été formatée et le contrôle final réussit. Ces sorties initiales sont conservées séparément des résultats verts.

## État Cargo et outils

Le garde Cargo passe ses neuf tests : aucun identifiant de licence ni fichier LICENSE n’est déclaré, la publication reste désactivée, le choix reste no_license_selected, les droits de redistribution restent not_assessed et le marqueur SPDX reste NOASSERTION. Cela décrit l’état contrôlé; aucune licence ou permission n’est attribuée.

Le test processus tourne sous Bubblewrap 0.9.0 avec namespaces réseau et PID isolés, compilation hors ligne/verrouillée, source et registre montés en lecture seule, limites CPU/mémoire/processus et aucun credential Forgejo. Rust/Cargo, Python, Git et leurs versions sont dans toolchain.txt. HEAD de base au début de la tranche : f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0; toutes les modifications restent non commitées. Les hashes des fichiers sources et du checkpoint sont dans source-hashes.txt et dans les manifestes append-only.

## Limites

Le service HTTP est une fixture loopback, pas Forgejo live. Forgejo v17 n’émet pas ici un commit SHA-256 réel; le test établit seulement que le parseur accepte la forme 64 caractères non nulle, pas qu’elle correspond à un objet Git. Le comportement live pour SHA-1 est couvert par d’autres parcours, sans provenance exacte du commit de build du binaire Forgejo. Le receiver/processus/CLI et le test ne font appel ni à AgentOS, ni à un modèle, ni à RAG/mémoire; aucun coût, merge, exécution d’un candidat, CI ou gain comparatif n’est observé. Aucun code de production Forgejo n’est copié ou modifié.

