//! Checked vector operations, independent of the native embedding runtime.

pub(crate) fn normalize(vector: &mut [f32]) -> Result<(), String> {
    if vector.is_empty() || vector.iter().any(|v| !v.is_finite()) {
        return Err("Empty or non-finite embedding".into());
    }
    let norm = vector.iter().map(|v| f64::from(*v).powi(2)).sum::<f64>().sqrt();
    if norm == 0.0 {
        return Err("Zero embedding".into());
    }
    for value in vector {
        *value = (f64::from(*value) / norm) as f32;
    }
    Ok(())
}

pub(crate) fn average(embeddings: &[Vec<f32>]) -> Result<Vec<f32>, String> {
    let dim = embeddings.first().ok_or("Empty embedding result")?.len();
    if dim == 0 || embeddings.iter().any(|v| v.len() != dim || v.iter().any(|x| !x.is_finite())) {
        return Err("Invalid embedding dimensions or values".into());
    }
    let mut mean = vec![0.0; dim];
    for i in 0..dim {
        mean[i] = (embeddings.iter().map(|v| f64::from(v[i])).sum::<f64>()
            / embeddings.len() as f64) as f32;
    }
    normalize(&mut mean)?;
    Ok(mean)
}

pub(crate) fn best_match<'a>(
    query: &[f32],
    vectors: impl IntoIterator<Item = &'a [f32]>,
) -> Result<(usize, f64), String> {
    if query.is_empty() || query.iter().any(|v| !v.is_finite()) {
        return Err("Invalid query embedding".into());
    }
    let mut best: Option<(usize, f64)> = None;
    for (index, vector) in vectors.into_iter().enumerate() {
        if vector.len() != query.len() || vector.iter().any(|v| !v.is_finite()) {
            return Err("Incompatible intent embedding; rebuild the cache".into());
        }
        let score = query.iter().zip(vector).map(|(a, b)| f64::from(*a) * f64::from(*b)).sum::<f64>();
        if best.is_none_or(|(_, old)| score > old) {
            best = Some((index, score.clamp(-1.0, 1.0)));
        }
    }
    best.ok_or_else(|| "No intent vectors available".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_model_outputs_are_errors() {
        assert!(average(&[]).is_err());
        assert!(average(&[vec![]]).is_err());
        assert!(average(&[vec![1.0], vec![1.0, 2.0]]).is_err());
        assert!(average(&[vec![f32::NAN]]).is_err());
        assert!(average(&[vec![0.0]]).is_err());
        assert!(best_match(&[1.0], std::iter::empty()).is_err());
        assert!(best_match(&[1.0], [vec![1.0, 2.0].as_slice()]).is_err());
    }

    #[test]
    fn finite_extreme_vectors_normalize_without_overflow() {
        let mean = average(&[vec![f32::MAX, 0.0], vec![f32::MAX, 0.0]]).unwrap();
        assert_eq!(mean, [1.0, 0.0]);
        assert_eq!(best_match(&mean, [vec![0.0, 1.0].as_slice(), mean.as_slice()]).unwrap(), (1, 1.0));
    }
}
