#!/bin/bash
# WiFi driver hardware test script
# Verifies WiFi driver works with real hardware

set -e

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

echo -e "${BLUE}=== WiFi Driver Hardware Test ===${NC}"

# Check if running as root for PCI passthrough
if [ "$EUID" -ne 0 ]; then
    echo -e "${YELLOW}Warning: Not running as root, skipping PCI passthrough tests${NC}"
    TEST_MODE="simulated"
else
    TEST_MODE="hardware"
fi

# Check available WiFi devices
check_wifi_devices() {
    echo -e "${BLUE}Checking for WiFi devices...${NC}"
    
    # Check for Intel iwlwifi devices
    if lspci | grep -q "Intel.*WiFi\|iwlwifi"; then
        echo -e "${GREEN}Found Intel iwlwifi device${NC}"
        echo "Details:"
        lspci | grep -i wifi | head -5
        WIFI_COUNT=$(lspci | grep -ci "wifi")
        echo -e "${GREEN}Found $WIFI_COUNT WiFi device(s)${NC}"
    elif lspci | grep -q "MediaTek.*WiFi"; then
        echo -e "${GREEN}Found MediaTek WiFi device${NC}"
        lspci | grep -i wifi | head -5
        WIFI_COUNT=$(lspci | grep -ci "wifi")
        echo -e "${GREEN}Found $WIFI_COUNT WiFi device(s)${NC}"
    elif lspci | grep -q "Realtek.*802\|RTL.*WiFi"; then
        echo -e "${GREEN}Found Realtek WiFi device${NC}"
        lspci | grep -i wifi | head -5
        WIFI_COUNT=$(lspci | grep -ci "wifi")
        echo -e "${GREEN}Found $WIFI_COUNT WiFi device(s)${NC}"
    else
        echo -e "${YELLOW}No WiFi device found on PCI bus${NC}"
        WIFI_COUNT=0
    fi
}

# Test WiFi driver registration
test_driver_registration() {
    echo -e "${BLUE}Testing WiFi driver registration...${NC}"
    
    # Check if driver modules are loaded
    if lsmod | grep -q "iwlwifi"; then
        echo -e "${GREEN}iwlwifi driver is loaded${NC}"
    else
        echo -e "${YELLOW}iwlwifi driver not loaded, attempting to load...${NC}"
        if modprobe iwlwifi 2>/dev/null; then
            echo -e "${GREEN}iwlwifi driver loaded successfully${NC}"
        else
            echo -e "${YELLOW}Failed to load iwlwifi driver${NC}"
        fi
    fi
    
    if lsmod | grep -q "rtw88"; then
        echo -e "${GREEN}rtw88 driver is loaded${NC}"
    else
        echo -e "${YELLOW}rtw88 driver not loaded, attempting to load...${NC}"
        if modprobe rtw88-usb 2>/dev/null || modprobe rtw88-pci 2>/dev/null; then
            echo -e "${GREEN}rtw88 driver loaded successfully${NC}"
        else
            echo -e "${YELLOW}Failed to load rtw88 driver${NC}"
        fi
    fi
    
    if lsmod | grep -q "mt7921"; then
        echo -e "${GREEN}mt7921 driver is loaded${NC}"
    else
        echo -e "${YELLOW}mt7921 driver not loaded, attempting to load...${NC}"
        if modprobe mt7921e 2>/dev/null; then
            echo -e "${GREEN}mt7921 driver loaded successfully${NC}"
        else
            echo -e "${YELLOW}Failed to load mt7921 driver${NC}"
        fi
    fi
}

# Test WiFi interface creation
test_interface() {
    echo -e "${BLUE}Testing WiFi interface...${NC}"
    
    # Check for wlan interfaces
    if ip link show wlan0 2>/dev/null; then
        echo -e "${GREEN}WiFi interface wlan0 found${NC}"
    elif ip link show wlp* 2>/dev/null; then
        echo -e "${GREEN}WiFi interface wlp* found${NC}"
    else
        echo -e "${YELLOW}No WiFi interface found${NC}"
        
        # Try to create with rfkill
        if rfkill list 2>/dev/null | grep -q "wifi"; then
            echo "WiFi interface is rfkill blocked"
            rfkill unblock wifi 2>/dev/null || rfkill unblock all 2>/dev/null
            echo "Attempted to unblock WiFi"
        fi
    fi
}

# Test WiFi scanning
test_wifi_scan() {
    echo -e "${BLUE}Testing WiFi scanning...${NC}"
    
    if ip link show wlan0 2>/dev/null || ip link show wlp* 2>/dev/null; then
        WIFI_IFACE=$(ip link show | grep -oP 'wlp\d+|wlan0' | head -1)
        
        echo "Scanning on interface: $WIFI_IFACE"
        
        # Bring up interface if down
        ip link set $WIFI_IFACE up 2>/dev/null || true
        
        # Scan for networks
        if command -v iwlist &> /dev/null; then
            echo "Using iwlist:"
            if iwlist $WIFI_IFACE scan 2>/dev/null | grep -q "ESSID"; then
                echo -e "${GREEN}Found access points:${NC}"
                iwlist $WIFI_IFACE scan 2>/dev/null | grep ESSID | head -5
            else
                echo -e "${YELLOW}No scan results available${NC}"
            fi
        elif command -v nmcli &> /dev/null; then
            echo "Using NetworkManager:"
            nmcli device wifi list 2>/dev/null | grep -E "^\*|SSID" | head -5 || echo -e "${YELLOW}No scans available${NC}"
        else
            echo -e "${YELLOW}No WiFi tools available for scanning${NC}"
        fi
    fi
}

# Test signal level and link quality
test_signal_quality() {
    echo -e "${BLUE}Testing signal quality...${NC}"
    
    if ip link show wlan0 2>/dev/null || ip link show wlp* 2>/dev/null; then
        WIFI_IFACE=$(ip link show | grep -oP 'wlp\d+|wlan0' | head -1)
        
        # Try to get signal quality using iw or cat /proc/net/wireless
        if [ -f /proc/net/wireless ]; then
            QUALITY=$(cat /proc/net/wireless 2>/dev/null | grep $WIFI_IFACE)
            if [ -n "$QUALITY" ]; then
                echo -e "${GREEN}Signal quality: $QUALITY${NC}"
            else
                echo -e "${YELLOW}No signal quality info available${NC}"
            fi
        fi
    fi
}

# Run the test
main() {
    echo -e "${BLUE}Starting WiFi driver hardware test${NC}"
    echo "Timestamp: $(date)"
    echo "Kernel: $(uname -r)"
    echo "================================"
    
    if [ "$TEST_MODE" = "simulated" ]; then
        echo -e "${YELLOW}Running in simulated mode (no root)${NC}"
    fi
    
    check_wifi_devices
    test_driver_registration
    test_interface
    test_wifi_scan
    test_signal_quality
    
    echo -e "${GREEN}WiFi driver test completed${NC}"
}

main "$@"