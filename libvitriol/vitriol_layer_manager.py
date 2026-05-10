#!/usr/bin/env python3
"""
VITRIOL Layer Manager Service
Manages on-demand layer loading from SSD to GPU VRAM

Architecture:
- Monitors GPU memory usage via nvidia-smi
- Tracks layer access patterns
- Preloads "hot" layers to GPU, keeps "cold" layers on SSD
- Uses llama.cpp's --gpu-layers to control VRAM usage
"""

import os
import time
import json
import subprocess
import threading
import requests
from collections import deque

class VitriolLayerManager:
    def __init__(self, model_path, llama_port=5002, max_gpu_layers=25):
        self.model_path = model_path
        self.llama_port = llama_port
        self.max_gpu_layers = max_gpu_layers
        
        # Layer access tracking
        self.layer_access_count = deque(maxlen=100)
        self.hot_layers = set()  # Layers that are frequently accessed
        self.cold_layers = set()  # Layers that are rarely accessed
        
        # Memory tracking
        self.gpu_memory_used = 0
        self.gpu_memory_total = 8112  # MB for GTX 1070 Ti
        
        # Configuration
        self.hot_threshold = 5  # Accesses to be considered "hot"
        self.cooldown_period = 60  # Seconds before re-evaluating
        
        self.running = False
        
    def get_gpu_memory(self):
        """Get current GPU memory usage via nvidia-smi"""
        try:
            result = subprocess.run(
                ['nvidia-smi', '--query-gpu=memory.used,memory.total', '--format=csv,noheader,nounits'],
                capture_output=True, text=True, timeout=5
            )
            used, total = result.stdout.strip().split(',')
            self.gpu_memory_used = int(used.strip())
            self.gpu_memory_total = int(total.strip())
            return self.gpu_memory_used, self.gpu_memory_total
        except Exception as e:
            print(f"Error getting GPU memory: {e}")
            return 0, 8112
    
    def analyze_inference(self):
        """Analyze recent inference patterns to identify hot layers"""
        # This is a simplified version - in production, we'd parse model architecture
        # For Qwen 3.5 9B with 32 layers:
        # - Early layers (0-10): Embedding, attention pre-processing
        # - Middle layers (11-22): Core transformer blocks  
        # - Late layers (23-31): Output projection, final norm
        
        # For now, assume first 15 layers are "hot" (initial processing)
        # and last 17 layers are "cold" (less frequent during generation)
        
        current_used, _ = self.get_gpu_memory()
        
        # Calculate how many layers we can fit
        # Each layer is roughly: (4096 * 4096 * 4) bytes for Q/K/V = 64MB
        # Plus feed-forward: (4096 * 12288 * 4) = 192MB
        # Total per layer ≈ 256MB
        
        layer_size_estimate = 200  # MB per layer (conservative)
        available_memory = self.gpu_memory_total - current_used - 500  # Reserve 500MB
        optimal_layers = min(max(1, available_memory // layer_size_estimate), self.max_gpu_layers)
        
        return optimal_layers
    
    def adjust_gpu_layers(self, num_layers):
        """Send request to llama.cpp to adjust GPU layers"""
        # Note: This requires llama.cpp to support dynamic layer adjustment
        # For now, we just log the recommendation
        print(f"VITRIOL: Recommending {num_layers} GPU layers (currently using {self.max_gpu_layers})")
        
        # In production, we'd restart llama.cpp with new -ngl value
        # or implement a hot-reload mechanism
        
        return num_layers
    
    def monitor_loop(self):
        """Main monitoring loop"""
        print(f"VITRIOL Layer Manager started")
        print(f"Model: {self.model_path}")
        print(f"Target llama.cpp on port {self.llama_port}")
        
        while self.running:
            try:
                # Check if llama.cpp is still running
                response = requests.get(f"http://localhost:{self.llama_port}/health", timeout=2)
                if response.status_code != 200:
                    print("llama.cpp not responding, waiting...")
                    time.sleep(5)
                    continue
                    
                # Get current GPU memory
                used, total = self.get_gpu_memory()
                print(f"GPU Memory: {used}/{total} MB ({100*used/total:.1f}%)")
                
                # Analyze and recommend layer count
                recommended_layers = self.analyze_inference()
                
                if recommended_layers != self.max_gpu_layers:
                    print(f"VITRIOL: Layer adjustment needed: {self.max_gpu_layers} -> {recommended_layers}")
                    # In production: trigger llama.cpp restart or hot-reload
                    
                # Wait before next check
                time.sleep(self.cooldown_period)
                
            except requests.exceptions.RequestException as e:
                print(f"Connection error: {e}")
                time.sleep(5)
            except Exception as e:
                print(f"Error in monitor loop: {e}")
                time.sleep(5)
    
    def start(self):
        """Start the layer manager"""
        self.running = True
        self.monitor_thread = threading.Thread(target=self.monitor_loop)
        self.monitor_thread.daemon = True
        self.monitor_thread.start()
        print("VITRIOL Layer Manager running in background")
    
    def stop(self):
        """Stop the layer manager"""
        self.running = False
        print("VITRIOL Layer Manager stopped")


if __name__ == "__main__":
    import sys
    
    model = "/mnt/data/ai/koboldcpp/Qwen_Qwen3.5-9B-Q4_K_M.gguf"
    port = 5004
    
    manager = VitriolLayerManager(model, port)
    
    try:
        manager.start()
        print("Press Ctrl+C to stop...")
        while True:
            time.sleep(1)
    except KeyboardInterrupt:
        print("\nShutting down...")
        manager.stop()