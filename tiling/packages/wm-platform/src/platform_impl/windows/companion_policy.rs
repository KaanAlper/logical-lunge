// Pure companion ancestry policy; runnable with rustc --test without desktop interaction.
pub(crate) fn is_descendant(pid: u32, ancestor: u32, parents: &std::collections::HashMap<u32, u32>) -> bool {
  if pid == 0 || ancestor == 0 || pid == ancestor { return false; }
  let mut current = pid;
  for _ in 0..8 {
    match parents.get(&current) {
      Some(&parent) if parent == ancestor => return true,
      Some(&parent) if parent != 0 && parent != current => current = parent,
      _ => return false,
    }
  }
  false
}

#[cfg(test)]
mod companion_tests {
  use std::collections::HashMap;

  use super::is_descendant;

  #[test]
  fn descendants_follow_the_parent_chain() {
    // generated trees: each of 8 processes has a lower id as its parent (0: none)
    let mut seed = 0x9E37_79B9_7F4A_7C15_u64;
    for _ in 0..500 {
      let mut parents = HashMap::new();
      for pid in 1..=8u32 {
        seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        #[allow(clippy::cast_possible_truncation)]
        parents.insert(pid, (seed >> 33) as u32 % pid);
      }
      for pid in 1..=8u32 {
        let mut ancestors = Vec::new();
        let mut current = pid;
        while let Some(&parent) = parents.get(&current).filter(|&&p| p != 0) {
          ancestors.push(parent);
          current = parent;
        }
        for ancestor in 1..=8u32 {
          assert_eq!(is_descendant(pid, ancestor, &parents), ancestors.contains(&ancestor), "{pid} / {ancestor} in {parents:?}");
        }
      }
    }
  }

  #[test]
  fn a_reused_parent_id_ends_the_walk() {
    let looped = HashMap::from([(1, 2), (2, 1), (5, 5)]);
    assert!(!is_descendant(1, 3, &looped));
    assert!(!is_descendant(5, 4, &looped));
    assert!(is_descendant(1, 2, &looped));
  }
}

#[cfg(test)]
mod bounds_tests {
  use super::*;
  use std::collections::HashMap;
  #[test]
  fn self_and_zero_are_not_ancestors_even_with_reused_ids() {
    let parents = HashMap::from([(1, 2), (2, 1), (3, 0)]);
    assert!(!is_descendant(1, 1, &parents));
    assert!(!is_descendant(3, 0, &parents));
    assert!(!is_descendant(0, 1, &parents));
  }
  #[test]
  fn unrelated_apps_and_chains_beyond_the_bound_are_excluded() {
    let parents = (1..=12).map(|pid| (pid, pid - 1)).collect::<HashMap<_, _>>();
    assert!(is_descendant(9, 1, &parents));
    assert!(!is_descendant(10, 1, &parents));
    assert!(!is_descendant(8, 12, &parents));
  }
}
