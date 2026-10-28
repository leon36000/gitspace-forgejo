# CONTINUATION-266 — provenance par livraison dans les sorties de réconciliation

Date : 27 octobre 2026. Worktree local fortement modifié, base HEAD f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0; aucune réinitialisation, suppression, publication ou modification de production. Les versions d’outils, commandes, sorties, empreintes de sources et manifestes sont conservés dans ce dossier.

## Changement

forgejo-reconcile-local ajoute sur les objets résultat single et les lignes résultat par livraison du batch :

- journal_source.sequence et journal_source.event_sha256;
- journal_source.receiver_attestation.status : verified, present_unverified, missing ou quarantined_not_checked;
- verification_scope, algorithm, key_id_sha256 et signature_included=false.

Une ligne n’est marquée verified qu’après le contrôle Ed25519 contre la politique de clés externes; les quarantaines sont explicitement non contrôlées. L’algorithme et le hash d’identifiant de clé ne sont publiés qu’en cas de vérification réussie. Aucune signature n’est exposée. Les sorties single/batch sont maintenant v13/v14 et --capabilities est v12. Le contrat indique que cela ne prouve ni vérité amont, ni correction du code, ni authenticité du writer local. Le HMAC brut n’est pas revérifié au rejeu.

Les validateurs Python des batch PR/commentaire et de sa reprise attendaient encore v13; les validateurs batch du replay natif en attendaient aussi v13. Ils ont été mis à v14. Les validateurs single restent à v13.

Le test single a d’abord révélé que journal_source.sequence était absent (null) du résultat; après ajout du champ, le test contractuel passe dans la suite du binaire. Le premier passage des harnesses Python a ensuite détecté les attentes batch obsolètes; ils ont été corrigés et les deux modules concernés passent au complet.

## Vérification

Les commandes de commands.txt ont toutes terminé avec le code 0. cargo test rapporte 159 tests du binaire réussis et les groupes de contrat sélectionnés 5 (2 ignorés pour ports fixes), 4, 3 et 5 tests réussis. Les suites Python rapportent 29, 30 et 131 tests réussis. cargo check --all-targets, rustfmt, py_compile, la garde de métadonnées Cargo et ses tests passent.

Le test de processus actual_reconcile_cli_process_reads_private_workflow_run_with_host_credential_fd exécute le receiver et le CLI Rust réels sous Bubblewrap, avec namespaces réseau et PID séparés; 1/1 test sélectionné passe. Il confirme la séquence, le hash d’événement, le statut vérifié, la portée receiver_signature_only, l’identifiant de clé haché et l’absence de signature publiée dans la sortie du processus enfant. Il n’a pas démarré Forgejo ni un agent LLM.

Un premier test des validateurs pendant l’intégration a échoué parce qu’ils attendaient le schéma batch précédent; après alignement single/batch, les modules concernés passent. Aucun résultat d’échec n’est masqué dans les journaux finaux de réussite; les sorties de la vérification finale sont hachées ici.

## Limites

- Une erreur d’enveloppe batch sans lignes par livraison n’expose pas une observation pour chaque entrée.
- Une signature Ed25519 atteste la provenance du receiver selon sa clé de confiance, pas la validité de la source amont, le comportement du receiver, la correction du logiciel ou l’identité d’un writer qui peut réécrire le journal.
- Le HMAC source n’est pas persisté/revérifié au replay; legacy sans clé reste non authentifié.
- Le test de processus est isolé et local; aucune instance Forgejo live ni vérificateur/merge n’est dans ce lot.
- Aucun LLM/AgentOS, gain comparatif, réduction de coût ou bénéfice général n’est démontré.
- Les routes et les politiques Forgejo externes ne sont pas réexaminées ici.
- L’état légal demeure license=null, license_file=null, publish=[], no_license_selected, not_assessed, NOASSERTION; GitSpace reste sans licence déterminée.

Suite indépendante : poursuivre les refus composés non démontrés, obtenir une preuve de contrôle indépendant des writers/credential de fusion, puis exécuter le protocole comparatif avec modèles et budgets autorisés. Le projet reste incomplet.
