use agent_forge_core::forgejo::{
    MergeWriterAdmissionErrorV1, MergeWriterAdmissionProfileV1, validate_merge_writer_admission_v1,
};

#[test]
fn local_sandbox_profile_requires_a_host_attestation_for_single_writer_exclusion() {
    assert_eq!(
        validate_merge_writer_admission_v1(
            MergeWriterAdmissionProfileV1::LocalSingleWriterSandbox,
            false,
        ),
        Err(MergeWriterAdmissionErrorV1::LocalSingleWriterSupervisorAttestationUnavailable)
    );
    assert_eq!(
        validate_merge_writer_admission_v1(
            MergeWriterAdmissionProfileV1::LocalSingleWriterSandbox,
            true,
        ),
        Ok(())
    );
}

#[test]
fn forgejo_v17_multiwriter_profile_is_refused_without_server_expected_base_cas() {
    assert_eq!(
        validate_merge_writer_admission_v1(
            MergeWriterAdmissionProfileV1::ForgejoV17MultiWriter,
            false,
        ),
        Err(MergeWriterAdmissionErrorV1::ServerExpectedBasePreconditionUnavailable)
    );
}
