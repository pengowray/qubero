/**
 * BooleanAddressResource.java
 *
 * @author Created by Omnicore CodeGuide
 */

package net.pengo.resource;

import java.io.IOException;
import java.io.InputStream;

import net.pengo.data.ArrayData;
import net.pengo.data.Data;
import net.pengo.data.SelectionData;
import net.pengo.pointer.JavaPointer;

public class BooleanAddressedResource extends BooleanResource implements AddressedResource {
    public final JavaPointer selResP = new JavaPointer("net.pengo.resource.SelectionResource"); //private SelectionResource selRes;
    public final JavaPointer rbitP = new JavaPointer("net.pengo.resource.IntResource"); //private IntResource rbit;
    
    /** Creates a new instance of BooleanResource */
    
    public BooleanAddressedResource(SelectionResource selRes) {
        this(selRes, new IntPrimativeResource(0));
    }
    public BooleanAddressedResource(SelectionResource selRes, IntResource rbit) {
        selResP.addSink(this);
        selResP.setName("Selection");
        
        rbitP.addSink(this);
        rbitP.setName("Bit index");
        setSelectionResource(selRes);
        setRbit(rbit);
    }
    
    public Resource[] getSources() {
	return new Resource[]{selResP,rbitP};
    }
    
    public net.pengo.restree.ResourceList getSubResources() {
        return null;
    }
    
    public void setRbit(IntResource rbit) {
	rbitP.setValue(rbit);
    }
    
    public IntResource getRbit() {
	return (IntResource)rbitP.evaluate();
    }
    
    public JavaPointer getRbitPointer() {
	return rbitP;
    }

    public SelectionResource getSelectionResource() {
        return (SelectionResource)selResP.evaluate();
    }
    
    public void setSelectionResource(SelectionResource selRes) {
        selResP.setValue(selRes);
    }
    
    public boolean isPrimative() {
	return false;
    }
    
    public boolean getValue() {
	try {
	    long whichBit = getRbit().toLong();
	    
	    long myByte = whichBit/8;
	    int myBit = (int)(whichBit%8);
	    
	    SelectionData selData = getSelectionResource().getSelectionData();
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
	    long whichBit = getRbit().toLong();
	    
	    int myByte = (int)whichBit/8; //xxx: shuold be long :(
	    int myBit = (int)(whichBit%8);
	    
	    SelectionData selData = getSelectionResource().getSelectionData();
	    //long start = selData.getStart();
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
	    //openFile.getEditableData().insertReplace(getSelectionResource().getSelection(), newdata);
	    getSelectionResource().insertReplace(newdata);
        } catch (java.io.IOException e) {
	    //fixme
	    e.printStackTrace();
	    return;
        }
        
    }
    
    /*
    public void editProperties() {
        new BooleanAddressedResourcePropertiesForm(this).show();
    }
    */
    
    public void doubleClickAction() {
        super.doubleClickAction();
        getSelectionResource().makeActive();
    }
    
}

