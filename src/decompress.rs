use ast::*;
use std::collections::HashMap;
use std::sync::Arc;

#[cfg(test)]
use debug::DebugDictionary;

pub struct Decompress {
    path_prefixes: HashMap<Subst, Arc<PathPrefix>>,
    abs_paths: HashMap<Subst, Arc<AbsolutePath>>,
    types: HashMap<Subst, Arc<Type>>,
    subst_counter: u64,
}

impl Decompress {
    fn alloc_subst<T, D>(&mut self, node: &Arc<T>, dict: D)
    where
        D: FnOnce(&mut Self) -> &mut HashMap<Subst, Arc<T>>,
        T: ::std::hash::Hash + Eq,
    {
        let subst = Subst(self.subst_counter);
        self.subst_counter += 1;
        dict(self).insert(subst, node.clone());
    }

    fn decompress_symbol(&mut self, symbol: &Symbol) -> Symbol {
        Symbol {
            name: self.decompress_abs_path(&symbol.name),
            instantiating_crate: symbol
                .instantiating_crate
                .as_ref()
                .map(|ic| self.decompress_path_prefix(ic)),
        }
    }

    fn decompress_abs_path(&mut self, abs_path: &Arc<AbsolutePath>) -> Arc<AbsolutePath> {
        match **abs_path {
            AbsolutePath::Path { ref name, ref args } => {
                let new_path_prefix = self.decompress_path_prefix(name);
                let decompressed_args = self.decompress_generic_parameter_list(args);

                let decompressed =
                    if Arc::ptr_eq(name, &new_path_prefix) && decompressed_args.ptr_eq(args) {
                        abs_path.clone()
                    } else {
                        Arc::new(AbsolutePath::Path {
                            name: new_path_prefix,
                            args: decompressed_args,
                        })
                    };

                if !args.is_empty() {
                    self.alloc_subst(&decompressed, |this| &mut this.abs_paths);
                }

                decompressed
            }

            AbsolutePath::Subst(ref subst) => {
                if let Some(abs_path) = self.abs_paths.get(subst) {
                    abs_path.clone()
                } else if let Some(prefix) = self.path_prefixes.get(subst) {
                    Arc::new(AbsolutePath::Path {
                        name: prefix.clone(),
                        args: GenericArgumentList::new_empty(),
                    })
                } else {
                    unreachable!()
                }
            }
        }
    }

    fn decompress_path_prefix(&mut self, path_prefix: &Arc<PathPrefix>) -> Arc<PathPrefix> {
        let decompressed = match **path_prefix {
            PathPrefix::CrateId { .. } => path_prefix.clone(),
            PathPrefix::TraitImpl {
                ref self_type,
                ref impled_trait,
                dis,
            } => {
                let decompressed_self_type = self.decompress_type(self_type);
                let decompressed_impled_trait = impled_trait.as_ref().map(|t| self.decompress_abs_path(t));

                Arc::new(PathPrefix::TraitImpl {
                    self_type: decompressed_self_type,
                    impled_trait: decompressed_impled_trait,
                    dis,
                })
            }
            PathPrefix::Node {
                ref prefix,
                ref ident,
            } => {
                let decompressed_prefix = self.decompress_path_prefix(prefix);

                if Arc::ptr_eq(prefix, &decompressed_prefix) {
                    path_prefix.clone()
                } else {
                    Arc::new(PathPrefix::Node {
                        prefix: decompressed_prefix,
                        ident: ident.clone(),
                    })
                }
            }
            PathPrefix::Subst(ref subst) => {
                // NOTE: We return here, that is, without allocating a
                //       substitution.
                return if let Some(prefix) = self.path_prefixes.get(subst) {
                    prefix.clone()
                } else {
                    unreachable!()
                };
            }
        };

        self.alloc_subst(&decompressed, |this| &mut this.path_prefixes);

        decompressed
    }

    fn decompress_generic_parameter_list(
        &mut self,
        compressed: &GenericArgumentList,
    ) -> GenericArgumentList {
        GenericArgumentList(compressed.iter().map(|t| self.decompress_type(t)).collect())
    }

    fn decompress_type(&mut self, compressed: &Arc<Type>) -> Arc<Type> {
        let decompressed = match **compressed {
            Type::BasicType(_) => {
                // Exit here!
                return compressed.clone();
            }
            Type::Ref(ref compressed_inner) => {
                let decompressed_inner = self.decompress_type(compressed_inner);

                if Arc::ptr_eq(compressed_inner, &decompressed_inner) {
                    compressed.clone()
                } else {
                    Arc::new(Type::Ref(decompressed_inner))
                }
            }
            Type::RefMut(ref compressed_inner) => {
                let decompressed_inner = self.decompress_type(compressed_inner);

                if Arc::ptr_eq(compressed_inner, &decompressed_inner) {
                    compressed.clone()
                } else {
                    Arc::new(Type::RefMut(decompressed_inner))
                }
            }
            Type::RawPtrConst(ref compressed_inner) => {
                let decompressed_inner = self.decompress_type(compressed_inner);

                if Arc::ptr_eq(compressed_inner, &decompressed_inner) {
                    compressed.clone()
                } else {
                    Arc::new(Type::RawPtrConst(decompressed_inner))
                }
            }
            Type::RawPtrMut(ref compressed_inner) => {
                let decompressed_inner = self.decompress_type(compressed_inner);

                if Arc::ptr_eq(compressed_inner, &decompressed_inner) {
                    compressed.clone()
                } else {
                    Arc::new(Type::RawPtrMut(decompressed_inner))
                }
            }
            Type::Array(opt_size, ref compressed_inner) => {
                let decompressed_inner = self.decompress_type(compressed_inner);

                if Arc::ptr_eq(compressed_inner, &decompressed_inner) {
                    compressed.clone()
                } else {
                    Arc::new(Type::Array(opt_size, decompressed_inner))
                }
            }
            Type::Tuple(ref compressed_components) => {
                let decompressed_components: Vec<_> = compressed_components
                    .iter()
                    .map(|t| self.decompress_type(t))
                    .collect();

                if decompressed_components
                    .iter()
                    .zip(compressed_components.iter())
                    .all(|(a, b)| Arc::ptr_eq(a, b))
                {
                    compressed.clone()
                } else {
                    Arc::new(Type::Tuple(decompressed_components))
                }
            }
            Type::Named(ref abs_path) => {
                let decompressed_abs_path = self.decompress_abs_path(abs_path);

                // Exit here!
                return if Arc::ptr_eq(abs_path, &decompressed_abs_path) {
                    compressed.clone()
                } else {
                    Arc::new(Type::Named(decompressed_abs_path))
                };
            }
            Type::Fn {
                is_unsafe,
                abi,
                ref return_type,
                ref params,
            } => {
                let decompressed_params: Vec<_> =
                    params.iter().map(|t| self.decompress_type(t)).collect();

                let decompressed_return_type =
                    return_type.as_ref().map(|t| self.decompress_type(t));

                let return_types_same = match (return_type, &decompressed_return_type) {
                    (Some(ref a), Some(ref b)) => Arc::ptr_eq(a, b),
                    (None, None) => true,
                    _ => unreachable!(),
                };

                if return_types_same && decompressed_params
                    .iter()
                    .zip(params.iter())
                    .all(|(a, b)| Arc::ptr_eq(a, b))
                {
                    compressed.clone()
                } else {
                    Arc::new(Type::Fn {
                        is_unsafe,
                        abi,
                        return_type: decompressed_return_type,
                        params: decompressed_params,
                    })
                }
            }
            Type::GenericParam(_) => compressed.clone(),
            Type::Subst(ref subst) => {
                return if let Some(t) = self.types.get(subst) {
                    t.clone()
                } else if let Some(abs_path) = self.abs_paths.get(subst) {
                    Arc::new(Type::Named(abs_path.clone()))
                } else if let Some(prefix) = self.path_prefixes.get(subst) {
                    Arc::new(Type::Named(Arc::new(AbsolutePath::Path {
                        name: prefix.clone(),
                        args: GenericArgumentList::new_empty(),
                    })))
                } else {
                    unreachable!()
                };
            }
        };

        self.alloc_subst(&decompressed, |this| &mut this.types);

        decompressed
    }
}

pub fn decompress_ext(symbol: &Symbol) -> (Symbol, Decompress) {
    let mut state = Decompress {
        abs_paths: HashMap::new(),
        path_prefixes: HashMap::new(),
        types: HashMap::new(),
        subst_counter: 0,
    };
    let decompressed = state.decompress_symbol(symbol);
    (decompressed, state)
}

#[cfg(test)]
impl Decompress {
    pub fn to_debug_dictionary(&self) -> DebugDictionary {
        use ast_demangle::AstDemangle;

        let mut items = vec![];

        items.extend(
            self.path_prefixes
                .iter()
                .map(|(&subst, ast)| (subst, ast.demangle(true))),
        );

        items.extend(
            self.abs_paths
                .iter()
                .map(|(&subst, ast)| (subst, ast.demangle(true))),
        );

        items.extend(
            self.types
                .iter()
                .map(|(&subst, ast)| (subst, ast.demangle(true))),
        );

        DebugDictionary::new(items)
    }
}
