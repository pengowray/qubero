/**
 * AddressResource.java
 *
 * @author Created by Omnicore CodeGuide
 */

package net.pengo.resource;

import net.pengo.app.*;
import net.pengo.selection.*;
import net.pengo.data.*;
import net.pengo.propertyEditor.*;

import java.util.*;

public class AddressResource extends Resource {
    private IntResource address;
    private IntResource length;
    
    private List dependantsList;
    
    AddressResource(OpenFile of, IntResource address, IntResource length) {
        super(of);
        this.address = address;
    }
    
    public LongListSelectionModel toSelection() {
        long firstIndex = address.toLong();
        long lastIndex = firstIndex + length.toLong() -1;
        return new SimpleLongListSelectionModel(firstIndex, lastIndex);
    }
    
    public String toString() {
        return "Pointer@" + Long.toString(address.getSelectionData().getStart(), 16) +" -> address:" + address.getValue().toString(16) + " = " + toSelection();
    }
}

