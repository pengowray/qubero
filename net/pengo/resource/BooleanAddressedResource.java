/**
 * BooleanAddressResource.java
 *
 * @author Created by Omnicore CodeGuide
 */

package net.pengo.resource;

import java.io.*;
import javax.swing.*;
import javax.swing.event.*;
import java.awt.event.ActionEvent;
import java.util.*;
import java.math.*;
import java.io.IOException;

import net.pengo.app.*;
import net.pengo.selection.*;
import net.pengo.data.*;
import net.pengo.propertyEditor.*;

public class BooleanAddressedResource extends BooleanResource implements AddressedResource
{
    private SelectionResource selRes;
    private IntResource rbit;
    
    /** Creates a new instance of BooleanResource */
    
    public BooleanAddressedResource(OpenFile of, SelectionResource selRes, IntResource rbit) {
        super(of);
        this.selRes = selRes;
        this.rbit = rbit;
    }
    
    public net.pengo.restree.ResourceList getSubResources() {
        return null;
    }
    
    public SelectionResource getSelectionResource() {
        return selRes;
    }

    public void setSelectionResource(SelectionResource selRes) {
        this.selRes = selRes;
    }
    
    public boolean isPrimative() {
	return false;
    }
        
    public boolean getValue() {
	try {
	    long whichBit = rbit.toLong();
	    
	    long myByte = whichBit/8;
	    int myBit = (int)(whichBit%8);
    
	    SelectionData selData = selRes.getSelectionData();
	    InputStream is = selData.dataStream();
	    is.skip(myByte);
	    byte[] valueByte = new byte[1];
	    is.read(valueByte);
    
	    boolean rValue = ((((int)valueByte[0]) & (1 << myBit)) >= 1);
	    return rValue;
	} catch (IOException e) {
	    //fixme
	    e.printStackTrace();
	    return false;
	}
    }
    
    public void setValue(boolean b) {
        try {
            long whichBit = rbit.toLong();

            int myByte = (int)whichBit/8; //xxx: shuold be long :(
            int myBit = (int)(whichBit%8);

            SelectionData selData = selRes.getSelectionData();
            long start = selData.getStart();
            //byte[] valueBytes = selData.readByteArray(start, myByte+1); //xxx: fingers crossed
            byte[] valueBytes = selData.readByteArray(); //xxx: shouldn't take the whole thing you pig

            /*
            InputStream is = selData.dataStream();
            is.skip(myByte);
            byte[] valueByte = new byte[1];
            is.read(valueByte);
            is.close();
            */

            if (b) {
                valueBytes[myByte] |= (1<<myBit);
            } else {
                valueBytes[myByte] &= ~(1<<myBit);
            }

            Data newdata = new ArrayData(valueBytes);
            //xxx: this is new.. must be changed in IntResource too
            openFile.getEditableData().insertReplace(selRes.getSelection(), newdata);
        } catch (java.io.IOException e) {
            //fixme
            e.printStackTrace();
            return;
        }
        
    }

    public void editProperties() {
	new BooleanAddressedResourcePropertiesForm(this).show();
    }
}

