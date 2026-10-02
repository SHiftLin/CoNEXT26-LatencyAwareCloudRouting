use std::cmp::max;

#[derive(Default)]
pub struct FastMap<V> {
    max_key: usize,
    cap: u32,
    map: Vec<V>,
}

impl<V: Default + Clone> FastMap<V> {
    pub fn new(cap: u32) -> Self {
        FastMap {
            max_key: 0,
            cap: cap,
            map: vec![Default::default(); cap as usize],
        }
    }

    fn update_max_key(&mut self, key: u32) {
        self.max_key = max(self.max_key, (key + 1) as usize);
    }

    pub fn keys(&self, filter: fn(&V) -> bool) -> Vec<u32> {
        let mut keys = Vec::new();
        for i in 0..self.max_key {
            if filter(unsafe { self.map.get_unchecked(i) }) {
                keys.push(i as u32);
            }
        }
        keys
    }

    pub fn values(&self) -> std::slice::Iter<V> {
        self.map[0..self.max_key].iter()
    }

    pub fn values_mut(&mut self) -> std::slice::IterMut<V> {
        self.map[0..self.max_key].iter_mut()
    }

    // The key used in insert/get method is not a reference (&u32),
    // because we explicitly let the caller know that the key should be integer-like
    pub fn insert_unsafe(&mut self, key: u32, value: V) -> V {
        self.update_max_key(key);
        std::mem::replace(unsafe { self.map.get_unchecked_mut(key as usize) }, value)
    }

    pub fn remove_unsafe(&mut self, key: u32) -> V {
        std::mem::replace(
            unsafe { self.map.get_unchecked_mut(key as usize) },
            Default::default(),
        )
    }

    pub fn get_unsafe(&self, key: u32) -> &V {
        unsafe { self.map.get_unchecked(key as usize) }
    }

    pub fn get_mut_unsafe(&mut self, key: u32) -> &mut V {
        self.update_max_key(key);
        unsafe { self.map.get_unchecked_mut(key as usize) }
    }

    pub fn insert(&mut self, key: u32, value: V) -> Option<V> {
        if key < self.cap {
            Some(self.insert_unsafe(key, value))
        } else {
            None
        }
    }

    pub fn remove(&mut self, key: u32) -> Option<V> {
        if key < self.cap {
            Some(self.remove_unsafe(key))
        } else {
            None
        }
    }

    pub fn get(&self, key: u32) -> Option<&V> {
        if key < self.cap {
            Some(self.get_unsafe(key))
        } else {
            None
        }
    }

    pub fn get_mut(&mut self, key: u32) -> Option<&mut V> {
        if key < self.cap {
            Some(self.get_mut_unsafe(key))
        } else {
            None
        }
    }
}

pub mod fast_map {
    pub struct Iter<'a, V: 'a> {
        pub(super) key: usize,
        pub(super) map: &'a [V],
    }

    impl<'a, V> Iterator for Iter<'a, V> {
        type Item = (u32, &'a V);

        fn next(&mut self) -> Option<Self::Item> {
            if self.key < self.map.len() {
                let item = (self.key as u32, &self.map[self.key]);
                self.key += 1;
                Some(item)
            } else {
                None
            }
        }
    }

    pub struct IterMut<'a, V: 'a> {
        pub(super) key: usize,
        pub(super) map: &'a mut [V],
    }

    impl<'a, V> Iterator for IterMut<'a, V> {
        type Item = (u32, &'a mut V);

        fn next(&mut self) -> Option<Self::Item> {
            if self.key < self.map.len() {
                // SAFETY: This is ok because ...
                let item = (self.key as u32, unsafe {
                    //&mut *(&mut self.map[self.key] as *mut V)
                    &mut *self.map.as_mut_ptr().offset(self.key as isize)
                });
                self.key += 1;
                Some(item)
            } else {
                None
            }
        }
    }
}

impl<'a, V> IntoIterator for &'a FastMap<V> {
    type Item = (u32, &'a V);
    type IntoIter = fast_map::Iter<'a, V>;

    fn into_iter(self) -> Self::IntoIter {
        fast_map::Iter {
            key: 0,
            map: &self.map[0..self.max_key],
        }
    }
}

impl<'a, V> IntoIterator for &'a mut FastMap<V> {
    type Item = (u32, &'a mut V);
    type IntoIter = fast_map::IterMut<'a, V>;

    fn into_iter(self) -> Self::IntoIter {
        fast_map::IterMut {
            key: 0,
            map: &mut self.map[0..self.max_key],
        }
    }
}
