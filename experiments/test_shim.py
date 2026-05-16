#!/usr/bin/env python3
"""
VITRIOL Test Client - Quick verification of the KoboldCPP integration

Usage:
    python3 test_shim.py              # Test VITRIOL shim health
    python3 test_shim.py rectify      # Test context rectification
    python3 test_shim.py infer        # Test full inference pipeline
"""

import json
import sys
import requests

VITRIOL_URL = "http://localhost:5010"
KOBOLD_URL = "http://localhost:5001"


def test_health():
    """Check if VITRIOL shim is running"""
    print("Testing VITRIOL shim health...")
    try:
        resp = requests.get(f"{VITRIOL_URL}/health", timeout=5)
        if resp.status_code == 200:
            data = resp.json()
            print(f"✓ VITRIOL Status: {data['status']}")
            print(f"  KoboldCPP: {data['koboldcpp']['status']}")
            print(f"  Max Context: {data['config']['max_context_tokens']} tokens")
            return True
        else:
            print(f"✗ Health check failed: {resp.status_code}")
            return False
    except requests.exceptions.ConnectionError:
        print("✗ VITRIOL shim not running on port 5005")
        print("  Start with: python3 vitriol_shim.py")
        return False
    except Exception as e:
        print(f"✗ Error: {e}")
        return False


def test_kobold_direct():
    """Test KoboldCPP directly (bypass VITRIOL)"""
    print("\nTesting KoboldCPP direct connection...")
    try:
        resp = requests.get(f"{KOBOLD_URL}/api/v1/info", timeout=5)
        if resp.status_code == 200:
            print(f"✓ KoboldCPP responding on port 5001")
            return True
        else:
            print(f"✗ KoboldCPP returned {resp.status_code}")
            return False
    except requests.exceptions.ConnectionError:
        print("✗ KoboldCPP not running on port 5001")
        print("  Start with: ./run_qwen.sh")
        return False


def test_rectification():
    """Test context rectification endpoint"""
    print("\nTesting context rectification...")
    
    # Simulate a bloated context (like the 389k character crash)
    test_messages = [
        {"role": "system", "content": "You are a helpful assistant."},
        {"role": "user", "content": "Hello" * 1000},  # Bloated message
        {"role": "assistant", "content": "Hi" * 500 + "<reasoning>detailed reasoning</reasoning>"},
        {"role": "user", "content": "How are you?"},
        {"role": "assistant", "content": "I'm good!"},
        {"role": "user", "content": "What's the weather?"},
        {"role": "assistant", "content": "Sunny!" * 200},
        {"role": "user", "content": "Thanks!"},
    ]
    
    try:
        resp = requests.post(
            f"{VITRIOL_URL}/rectify",
            json={"messages": test_messages},
            timeout=5
        )
        
        if resp.status_code == 200:
            data = resp.json()
            stats = data['stats']
            print(f"✓ Rectification successful")
            print(f"  Messages: {data['original_messages']} -> {data['rectified_messages']}")
            print(f"  Tokens: {stats['original_tokens']} -> {stats['rectified_tokens']}")
            print(f"  Reduction: {stats['reduction_percent']:.1f}%")
            print(f"  Metadata stripped: {stats['metadata_stripped']}")
            return True
        else:
            print(f"✗ Rectification failed: {resp.status_code}")
            return False
    except Exception as e:
        print(f"✗ Error: {e}")
        return False


def test_inference():
    """Test full inference through VITRIOL"""
    print("\nTesting full inference pipeline...")
    
    test_messages = [
        {"role": "system", "content": "You are a helpful coding assistant."},
        {"role": "user", "content": "Write a Python function to calculate fibonacci numbers."}
    ]
    
    try:
        resp = requests.post(
            f"{VITRIOL_URL}/v1/chat/completions",
            json={
                "messages": test_messages,
                "max_tokens": 50,
                "temperature": 0.7
            },
            timeout=30
        )
        
        if resp.status_code == 200:
            data = resp.json()
            if 'choices' in data and len(data['choices']) > 0:
                content = data['choices'][0]['message']['content']
                print(f"✓ Inference successful")
                print(f"  Response: {content[:100]}...")
                return True
            else:
                print(f"✗ Unexpected response format: {data}")
                return False
        else:
            print(f"✗ Inference failed: {resp.status_code}")
            print(f"  Response: {resp.text[:200]}")
            return False
    except requests.exceptions.Timeout:
        print("✗ Inference timed out (model may still be loading)")
        return False
    except Exception as e:
        print(f"✗ Error: {e}")
        return False


def main():
    print("=== VITRIOL Integration Test ===\n")
    
    # Step 1: Check KoboldCPP
    kobold_ok = test_kobold_direct()
    
    # Step 2: Check VITRIOL shim
    vitriol_ok = test_health()
    
    if not vitriol_ok:
        print("\n✗ VITRIOL shim is not running. Cannot continue tests.")
        sys.exit(1)
    
    # Step 3: Test rectification
    rectify_ok = test_rectification()
    
    # Step 4: Test inference (only if Kobold is running)
    if kobold_ok:
        infer_ok = test_inference()
    else:
        print("\nSkipping inference test (KoboldCPP not running)")
        infer_ok = False
    
    # Summary
    print("\n=== Test Summary ===")
    print(f"KoboldCPP:    {'✓' if kobold_ok else '✗'}")
    print(f"VITRIOL:      {'✓' if vitriol_ok else '✗'}")
    print(f"Rectification: {'✓' if rectify_ok else '✗'}")
    print(f"Inference:    {'✓' if infer_ok else '✗'}")
    
    if vitriol_ok and rectify_ok:
        print("\n✓ VITRIOL is ready to rectify context for OpenCode!")
        print("  Point OpenCode to: http://localhost:5010/v1/chat/completions")
    
    sys.exit(0 if vitriol_ok else 1)


if __name__ == '__main__':
    main()
