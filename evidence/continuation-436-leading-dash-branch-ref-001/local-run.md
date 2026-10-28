# CONTINUATION-436 — lecture de refs de branche commençant par un tiret — 28 octobre 2026

## Résultat

Le même validateur **valid_branch** servait aux portées de fusion locales et aux noms de branche projetés dans les webhooks et les routes API. Il refuse les préfixes **-** pour éviter l’interprétation comme option de ligne de commande; son réemploi rejetait aussi le nom Git valide **-release-v1** lors d’une simple lecture HTTP.

Le code sépare désormais **valid_branch_ref_name** (validation de ref pour webhook/API/URL) et **valid_branch** (règle plus stricte conservée pour les portées locales susceptibles d’atteindre une commande). La réconciliation, la politique de routes canoniques et les URL de lecture utilisent le validateur de ref. Le corpus local de **git check-ref-format** accepte **refs/heads/-release-v1**; il rejette les exemples de traversal, de slash doublé, de segment pointé et de suffixe **.lock**.

## Vérifications et preuve

- Tests rouges avant correction : **push_notification_reconciles_git_valid_leading_dash_branch_ref** a été refusé comme **UnsupportedEvent**; **reconciliation_policy_allows_only_canonical_encoded_branch_segments** a refusé la route **/branches/-release-v1**.
- Un premier sélecteur **--exact** incomplet n’a exécuté aucun test. Sa sortie est conservée dans **route-policy-no-match.log**/**.exit** et n’est pas comptée comme résultat produit. Le sélecteur avec le chemin complet du module a ensuite exécuté le test.
- Tests ciblés verts : contrat de réconciliation 1/1, politique de route 1/1, adaptateur loopback 1/1.
- L’adaptateur réel de réconciliation a émis **GET /api/v1/repos/forge-admin/agent-forge-core/branches/-release-v1**; le test vérifie le readback de la SHA, l’URL canonique et l’absence d’en-tête **Authorization**.
- Suite Rust existante, commande **cargo test --offline --locked --all-targets -- --test-threads=1** : 38 groupes, 765 réussis, 0 échec, 200 ignorés. Les tests ignorés ne sont pas réputés exécutés par ce run.
- **cargo fmt --all -- --check**, **git diff --check**, **git diff --cached --check** et la garde de licence Cargo réussissent; cette dernière compte 12 tests. **cargo metadata** conserve **license=null**, **license_file=null**, **publish=[]**, **no_license_selected**, **redistribution_rights=not_assessed**, **NOASSERTION**.
- Le manifeste de run vérifie toutes ses empreintes locales; le vérificateur append-only global passe sur 13 622 chemins uniques (0 absent, divergent, non haché ou non sûr). Il conserve 1 enregistrement historique non haché et ignore 6 lignes vides.

## Portée et limites

L’adaptateur utilise un serveur HTTP loopback de test et le vrai processus Rust; il ne contacte pas une instance Forgejo. Le test prouve la construction de la requête et le comportement de l’adaptateur sur la route ciblée, pas l’acceptation par tous les serveurs, proxys ou noms Unicode. Les appels à **AgentRuntimeV1** restent des fixtures sous tests; aucun AgentOS, fournisseur LLM, RAG ou moteur de mémoire n’a été appelé. Aucun merge, benchmark, coût externe ou publication n’est établi par ce lot.

Les deux sorties rouges et les sources complètes sont dans ce dossier. Le corpus Git et l’audit d’interface sont **git-refname-oracle.json** et **runtime-interface-audit.txt**; la commande, le toolchain, les statuts et les empreintes de source sont également conservés. Le projet reste incomplet.

Avant synchronisation vers le dépôt privé, un balayage de motifs connus a examiné 79 fichiers texte candidats, hors rapport de balayage : aucune correspondance GitLab/GitHub PAT, clé OpenAI/AWS, clé privée PEM, Bearer long ou URL avec mot de passe. Ce contrôle est ciblé, non exhaustif. Les caches, runtime, configuration Claude locale et archives binaires restent hors transfert.
