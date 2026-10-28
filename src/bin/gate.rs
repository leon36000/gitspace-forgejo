use agent_forge_core::{
    ExpectedBinding, ProofEvidence, ProofTargetV2, SignedProof, TargetBoundProofV2, sign_proof,
    sign_target_bound_proof_v2, verify_proof, verify_target_bound_proof_v2,
};
use std::fs;
use std::path::Path;
use std::process::ExitCode;

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    match args.as_slice() {
        [_, operation, key_path, input_path, output_path] if operation == "sign" => {
            let key = fs::read(key_path).map_err(|e| format!("read verifier key: {e}"))?;
            let evidence: ProofEvidence =
                serde_json::from_slice(&fs::read(input_path).map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())?;
            let proof = sign_proof(evidence, &key)?;
            let bytes = serde_json::to_vec_pretty(&proof).map_err(|e| e.to_string())?;
            fs::write(output_path, bytes).map_err(|e| e.to_string())?;
            println!("signed {}", output_path);
            Ok(())
        }
        [
            _,
            operation,
            key_path,
            evidence_path,
            target_path,
            output_path,
        ] if operation == "sign-target-v2" => {
            let key = fs::read(key_path).map_err(|e| format!("read verifier key: {e}"))?;
            let evidence: ProofEvidence =
                serde_json::from_slice(&fs::read(evidence_path).map_err(|e| e.to_string())?)
                    .map_err(|e| format!("read V1 evidence: {e}"))?;
            let target: ProofTargetV2 =
                serde_json::from_slice(&fs::read(target_path).map_err(|e| e.to_string())?)
                    .map_err(|e| format!("read canonical target: {e}"))?;
            let proof = sign_target_bound_proof_v2(evidence, target, &key)?;
            let bytes = serde_json::to_vec_pretty(&proof).map_err(|e| e.to_string())?;
            fs::write(output_path, bytes).map_err(|e| e.to_string())?;
            println!("signed target-bound V2 proof {}", output_path);
            Ok(())
        }
        [_, operation, key_path, proof_path, expected_path] if operation == "check" => {
            let key = fs::read(key_path).map_err(|e| format!("read verifier key: {e}"))?;
            let proof: SignedProof =
                serde_json::from_slice(&fs::read(proof_path).map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())?;
            let expected: ExpectedBinding =
                serde_json::from_slice(&fs::read(expected_path).map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())?;
            verify_proof(Some(&proof), &key, &expected)
                .map_err(|e| format!("merge denied: {e:?}"))?;
            println!("PASS: proof signature, passing outcome and all current bindings match");
            Ok(())
        }
        [
            _,
            operation,
            key_path,
            proof_path,
            target_path,
            expected_path,
        ] if operation == "check-target-v2" => {
            let key = fs::read(key_path).map_err(|e| format!("read verifier key: {e}"))?;
            let proof: TargetBoundProofV2 =
                serde_json::from_slice(&fs::read(proof_path).map_err(|e| e.to_string())?)
                    .map_err(|e| format!("read target-bound V2 proof: {e}"))?;
            let target: ProofTargetV2 =
                serde_json::from_slice(&fs::read(target_path).map_err(|e| e.to_string())?)
                    .map_err(|e| format!("read expected target: {e}"))?;
            let expected: ExpectedBinding =
                serde_json::from_slice(&fs::read(expected_path).map_err(|e| e.to_string())?)
                    .map_err(|e| format!("read expected binding: {e}"))?;
            verify_target_bound_proof_v2(Some(&proof), &key, &target, &expected)
                .map_err(|e| format!("merge denied: {e:?}"))?;
            println!("PASS: target-bound V2 signature, target and current bindings match");
            Ok(())
        }
        [_, operation, key_path, input_path] if operation == "check-missing" => {
            let _key = fs::read(key_path).map_err(|e| e.to_string())?;
            let expected: ExpectedBinding =
                serde_json::from_slice(&fs::read(input_path).map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())?;
            verify_proof(None, b"unreachable", &expected)
                .map_err(|e| format!("merge denied: {e:?}"))
        }
        _ => Err(format!(
            "usage: {} sign <key> <evidence.json> <proof.json> | sign-target-v2 <key> <evidence.json> <target.json> <proof-v2.json> | check <key> <proof.json> <expected.json> | check-target-v2 <key> <proof-v2.json> <target.json> <expected.json> | check-missing <key> <expected.json>",
            Path::new(args.first().map(String::as_str).unwrap_or("gate")).display()
        )),
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::from(2)
        }
    }
}
