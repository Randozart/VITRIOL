"""
VITRIOL Vector Store - Lightweight Semantic Search
Optimized for: GTX 1070 Ti + i7-3770 (no AVX2)

Strategy: 
- Use pre-computed embeddings (cached on SSD)
- FAISS for fast similarity search
- Lazy loading to minimize RAM
"""

import json
import os
import hashlib
from typing import List, Dict, Any, Optional
from pathlib import Path

try:
    import faiss
    FAISS_AVAILABLE = True
except ImportError:
    FAISS_AVAILABLE = False
    print("FAISS not available, falling back to keyword search")

try:
    from sentence_transformers import SentenceTransformer
    SENTENCE_TRANSFORMERS_AVAILABLE = True
except ImportError:
    SENTENCE_TRANSFORMERS_AVAILABLE = False
    print("Sentence-transformers not available")


class VectorStore:
    """Lightweight vector store for context streaming"""
    
    def __init__(
        self,
        storage_path: str = "/tmp/vitriol_vector_db",
        embedding_model: str = "all-MiniLM-L6-v2",
        use_gpu: bool = False
    ):
        self.storage_path = Path(storage_path)
        self.storage_path.mkdir(parents=True, exist_ok=True)
        
        self.index_path = self.storage_path / "index.faiss"
        self.metadata_path = self.storage_path / "metadata.json"
        self.embeddings_path = self.storage_path / "embeddings.npy"
        
        self.index = None
        self.metadata = []
        self.embeddings = None
        
        # Lazy load model only when needed
        self.model = None
        self.embedding_model = embedding_model
        self.use_gpu = use_gpu and FAISS_AVAILABLE
        
    def _load_model(self):
        """Lazy load embedding model"""
        if self.model is None and SENTENCE_TRANSFORMERS_AVAILABLE:
            self.model = SentenceTransformer(self.embedding_model)
            print(f"Loaded embedding model: {self.embedding_model}")
    
    def _compute_embedding(self, text: str) -> List[float]:
        """Compute embedding for text"""
        self._load_model()
        if self.model:
            return self.model.encode(text).tolist()
        return []
    
    def add_chunks(self, chunks: List[Dict[str, Any]]) -> int:
        """
        Add text chunks to vector store
        Returns number of chunks added
        """
        if not chunks:
            return 0
        
        # Load existing metadata
        if self.metadata_path.exists():
            with open(self.metadata_path, 'r') as f:
                self.metadata = json.load(f)
        
        # Compute embeddings for new chunks
        new_embeddings = []
        new_metadata = []
        
        for chunk in chunks:
            chunk_id = hashlib.md5(
                f"{chunk.get('id', '')}{chunk.get('text', '')[:100]}".encode()
            ).hexdigest()
            
            # Skip if already indexed
            if any(m.get('id') == chunk_id for m in self.metadata):
                continue
            
            embedding = self._compute_embedding(chunk.get('text', ''))
            if embedding:
                new_embeddings.append(embedding)
                new_metadata.append({
                    'id': chunk_id,
                    'text': chunk.get('text', '')[:500],  # Store preview
                    'messages': chunk.get('messages', []),
                    'start_idx': chunk.get('start_idx', 0),
                    'end_idx': chunk.get('end_idx', 0)
                })
        
        if not new_embeddings:
            return 0
        
        # Build or update FAISS index
        import numpy as np
        new_embeddings_np = np.array(new_embeddings, dtype='float32')
        
        if self.index is None:
            # Create new index
            dimension = new_embeddings_np.shape[1]
            self.index = faiss.IndexFlatIP(dimension)  # Inner product for cosine similarity
            
            if self.use_gpu:
                print("Moving index to GPU...")
                self.index = faiss.index_cpu_to_all_gpus(self.index)
            
            self.index.add(new_embeddings_np)
        else:
            # Add to existing index
            self.index.add(new_embeddings_np)
        
        # Update metadata
        self.metadata.extend(new_metadata)
        
        # Save to disk
        self._save()
        
        print(f"Added {len(new_metadata)} chunks to vector store (total: {len(self.metadata)})")
        return len(new_metadata)
    
    def search(
        self,
        query: str,
        top_k: int = 3,
        threshold: float = 0.3
    ) -> List[Dict[str, Any]]:
        """
        Search for relevant chunks
        Returns list of chunks sorted by relevance
        """
        if self.index is None or not self.metadata:
            return []
        
        # Compute query embedding
        query_embedding = self._compute_embedding(query)
        if not query_embedding:
            return []
        
        import numpy as np
        query_embedding_np = np.array([query_embedding], dtype='float32')
        
        # Normalize for cosine similarity
        faiss.normalize_L2(query_embedding_np)
        
        # Search
        scores, indices = self.index.search(query_embedding_np, min(top_k, len(self.metadata)))
        
        # Filter by threshold and return results
        results = []
        for score, idx in zip(scores[0], indices[0]):
            if score >= threshold and idx < len(self.metadata):
                results.append({
                    'score': float(score),
                    **self.metadata[idx]
                })
        
        return results
    
    def _save(self):
        """Save index and metadata to disk"""
        if self.index:
            if self.use_gpu:
                # Move back to CPU for saving
                cpu_index = faiss.index_gpu_to_cpu(self.index)
                faiss.write_index(cpu_index, str(self.index_path))
            else:
                faiss.write_index(self.index, str(self.index_path))
        
        with open(self.metadata_path, 'w') as f:
            json.dump(self.metadata, f, indent=2)
    
    def load(self):
        """Load index and metadata from disk"""
        if self.index_path.exists() and FAISS_AVAILABLE:
            self.index = faiss.read_index(str(self.index_path))
            
            if self.use_gpu:
                print("Moving index to GPU...")
                self.index = faiss.index_cpu_to_all_gpus(self.index)
            
            print(f"Loaded FAISS index with {self.index.ntotal} vectors")
        
        if self.metadata_path.exists():
            with open(self.metadata_path, 'r') as f:
                self.metadata = json.load(f)
            print(f"Loaded {len(self.metadata)} metadata entries")


# Singleton instance for VITRIOL
_vector_store: Optional[VectorStore] = None


def get_vector_store() -> VectorStore:
    """Get or create vector store instance"""
    global _vector_store
    if _vector_store is None:
        _vector_store = VectorStore(
            storage_path="/tmp/vitriol_vector_db",
            use_gpu=False  # Set to True if you have GPU memory to spare
        )
        _vector_store.load()
    return _vector_store


def stream_context_with_vector(query: str, top_k: int = 3) -> List[Dict[str, Any]]:
    """
    Stream relevant context using vector search
    Drop-in replacement for stream_relevant_context()
    """
    vector_store = get_vector_store()
    results = vector_store.search(query, top_k=top_k)
    
    # Convert results back to messages
    streamed_messages = []
    for result in results:
        streamed_messages.extend(result.get('messages', []))
    
    return streamed_messages
