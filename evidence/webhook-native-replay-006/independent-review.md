# Revue indépendante de WEBHOOK-NATIVE-REPLAY-006

Revue en lecture seule du runner, du test et des artefacts du run 006. Le réviseur n'a lancé ni test ni service et n'a modifié aucun fichier.

Aucun défaut matériel n'a été relevé dans le nouvel invariant. validate_proxy_delivery_attempts lie dans l'ordre chaque requête à la tâche Forgejo par UUID, type d'événement et statut; il vérifie aussi la longueur et exige l'égalité du SHA-256 du corps brut reçu par le proxy avec celui du payload_content de la tâche. Les trois requêtes des artefacts correspondent aux trois tâches livrées, avec un hash commun. La quatrième tâche, créée après désactivation du hook, ne produit pas de requête. Les tests couvrent un hash altéré et une requête dupliquée.

La revue a initialement relevé un défaut de complétude des manifestes : le manifeste interne ne listait pas les nouveaux journaux de tests et de compilation, et le manifeste global n'avait pas encore d'entrée run006. Les deux manifestes ont été régénérés après l'ajout des artefacts. Le relevé de la fenêtre de reprise est limité explicitement à une seconde et n'établit pas l'absence de reprises après ce délai.

Limites : examen statique et des artefacts seulement; le réviseur n'a pas vérifié indépendamment l'exécution des tests. Proxy limité aux POST JSON du scénario, run n=1 et binaire local non relié au commit source Forgejo audité.
