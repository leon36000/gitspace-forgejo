# MISSION-012-SANDBOX-RESTART-001 — reprise après effet sous Bubblewrap

Date : 25 octobre 2026. Exécution locale isolée; aucun code produit Forgejo modifié. Le checkout produit utilisé pour construire les hôtes avait `HEAD=f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0` et un worktree déjà fortement modifié; ces modifications sont conservées. Les hashes sources exacts figurent dans le dossier de preuve.

## Résultat observé

Une admission webhook est faite contre Forgejo `17.0.0+gitea-1.22.0` local dans l’instance jetable. Le runner injecte un SIGKILL au vrai binaire `mission-012-mcp-host` après l’effet d’écriture; le premier passage de l’adaptateur Bubblewrap stdio se termine avec code 1, sans timeout ni sortie excessive, et l’événement MCP indique que l’hôte a fermé son flux de réponse. Le runner relance ensuite le host de confiance; l’adaptateur reconnecte à celui-ci via MCP stdio.

Avant redémarrage, le runner confirme exactement un intent d’écriture durable et aucun résultat. Le fichier candidat porte les octets post-effet attendus (`src/lib.rs`, SHA-256 `173304ad7d32aabc1e8e1ee41eabda3398a8690798e68f906bcebc73cbcd6d15`). Après attente de l’expiration du bail (10 250 ms), le nouvel hôte reçoit le même travail d’opération sous un fencing token supérieur : `1 → 2`. Il résout l’intent d’origine exactement une fois et produit un résultat. Le checkpoint est repris comme `unverified`; l’adaptateur rejoue l’opération exacte via MCP, voit l’échec rouge puis le test protégé vert, et obtient le verdict `candidate_ready_for_independent_verification`. Le segment de reprise contient huit reçus; la chaîne combinée en contient treize.

L’admission lie le travail à la base `5982815302165da6043bfba334199d3f1d028ec0` et à l’exigence SHA-256 `3a3c444075c93a3bcfb2bec17d1a2e74ec0051effc2994e6b02ac095eb0dd431`. Le dossier d’admission a un manifeste local vérifié 10/10 : [preuves d’admission](../mission-012-live-agent-webhook-sandbox-restart-001-admission/).

## Reproduction et provenance

Les commandes sont dans [`commands.txt`](commands.txt); versions et empreintes des exécutables dans [`toolchain.txt`](toolchain.txt). Le résumé [`agent-summary.json`](agent-summary.json) est une projection des champs utiles; les chemins privés, la configuration MCP, les journaux complets et toute donnée de credential ont été exclus. [`crash-event.json`](crash-event.json) ne conserve que le message d’erreur du flux. Les empreintes des runners, hôtes et tests sont dans [`source-hashes.txt`](source-hashes.txt).

Le runner vérifie explicitement le code de sortie non nul du premier passage, les octets exacts après effet, l’intent unique sans résultat, l’expiration du bail, la liaison de l’opération à un nouveau fence supérieur et l’unique résultat durable. La seconde invocation repasse par le mode Bubblewrap stdio. La source du runner et les assertions déterminantes sont incluses dans les empreintes.

## Limites

Effectif `n=1`. L’exécutant est `local-test-adapter`, pas un LLM; l’appel modèle est `false`, les compteurs de tokens sont indisponibles et le coût rapporté est nul parce qu’aucun modèle n’a été appelé. Le plafond configuré de 0,01 USD n’est pas une dépense ni une estimation. Ce lot s’arrête à la candidate vérifiable : aucune PR ni fusion n’a été créée dans ce run. Le parcours PR/fusion existant n’est pas rejoué.

Cela exerce un crash injecté après effet, un bail et le journal de cette implémentation sur un hôte local; cela ne démontre pas la récupération après perte d’alimentation, un hôte hostile ou multi-tenant, un processus hostile du même UID, une autorisation AgentOS/LLM, une sûreté générale ni un gain comparatif. L’empreinte établit l’identité des octets de preuve, pas la correction du logiciel.
