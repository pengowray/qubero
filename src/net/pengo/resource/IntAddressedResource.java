/**
 * IntAddressedResource.java
 *
 * @author Created by Omnicore CodeGuide
 */

package net.pengo.resource;

import java.io.IOException;
import java.math.BigInteger;
import java.util.Arrays;

import net.pengo.data.ArrayData;
import net.pengo.data.Data;
import net.pengo.data.SelectionData;
import net.pengo.dependency.QNode;
import net.pengo.pointer.JavaPointer;
import net.pengo.pointer.SmartPointer;
import net.pengo.propertyEditor.IntAddressedResourcePropertiesForm;

public class IntAddressedResource extends IntResource implements AddressedResource {
    private SmartPointer signedResP = new JavaPointer("net.pengo.resource.IntResource"); //private IntResource signedRes;
    private SmartPointer selResP = new JavaPointer("net.pengo.resource.SelectionResource"); //private SelectionResource selRes;
    private SmartPointer allowResizeP  = new JavaPointer("net.pengo.resource.BooleanResource");
    
    //fixme: separate "allowShrink" and "allowGrow"?
    //fixme: not to mention "grow by 4 bytes" etc
    
    //private BooleanResource allowResize = new BooleanPrimativeResource(getOpenFile(), false);
    
    public boolean isPrimative() {
        return false;
    }
    
    public IntAddressedResource(SelectionResource selRes) {
        this(selRes, IntResource.TWOS_COMP);
    }
    
    public IntAddressedResource(SelectionResource selRes, IntResource signedRes) {
        super();
        
        signedResP.setName("Signed");
        selResP.setName("Selection");
        allowResizeP.setName("Allow resize");
        
        signedResP.addSink(this);
        selResP.addSink(this);
        allowResizeP.addSink(this);
        
        setSelectionResource(selRes);
        setSignedRes(signedRes);
        setAllowResize(new BooleanPrimativeResource(false));
        
        //new IntAddressedResourcePropertiesForm(this).show();
    }
    
    public IntAddressedResource(SelectionResource selRes, int signed) {
        this(selRes, new IntPrimativeResource((long)signed));
    }
    
    public QNode[] getSources() {
        return new QNode[]{signedResP, selResP, allowResizeP};
    }
    
    //fixme: does this make the above redundant? this is dodgy.
    public JavaPointer[] getJPointers() {
        return new JavaPointer[]{(JavaPointer)signedResP, (JavaPointer)selResP, (JavaPointer)allowResizeP};
    }
    
    public SelectionResource getSelectionResource() {
        return (SelectionResource)selResP.evalute();
    }
    
    public void setSelectionResource(SelectionResource selRes) {
        selResP.setValue(selRes);
    }
    
    public IntResource getSignedRes() {
        return (IntResource)signedResP.evalute();
    }
    
    public void setSignedRes(IntResource signedRes) {
        signedResP.setValue(signedRes);
    }
    
    public void setSigned(int signage) {
        //fixme: allow proper conversion between signage types?
        
        setSignedRes( new IntPrimativeResource(signage) );
    }
    
    public int getSigned() {
        return getSignedRes().getValue().intValue();
    }
    
    public BooleanResource getAllowResize() {
        return (BooleanResource)allowResizeP.evalute();
    }
    
    public void setAllowResize(BooleanResource allowResize) {
        allowResizeP.setValue(allowResize);
    }
    
    public void doubleClickAction() {
        // duplicated in BooleanAddrssdResource
        super.doubleClickAction();
        getSelectionResource().makeActive();
    }
    
    public BigInteger getValue() {
        //byte[] data = sel.getDataStreamAsArray();
        int signed = getSigned();
        
        try {
            byte[] data = getSelectionResource().getSelectionData().readByteArray();
            if (data.length == 0) {
                return BigInteger.ZERO;
            }
            if (signed == UNSIGNED) {
                return new BigInteger(1, data);
            }
            else if (signed == ONES_COMP) {
                if ((data[0] & 0x80) > 0) {
                    byte[] data2 = new byte[data.length];
                    //int signum = -1;
                    data2[0] = (byte)((~data[0]) & 0x7f); // cut off highest bit and invert rest
                    for (int i=1; i<data.length; i++) {
                        data2[i] = (byte)~data[i];
                    }
                    return new BigInteger(-1, data2);
                }
                else {
                    // ones comp, positive.
                    return new BigInteger(1, data);
                }
            }
            else if (signed == TWOS_COMP) {
                // big-endian, two's comp.
                
                //remove padding
                if (data.length >= 2) {
                    int padcount=0;
                    //while (padcount < data.length && data[padcount] == (byte)0xFF) {
                    while (padcount < data.length && data[padcount] == -1) {
                        padcount++;
                    }
                    if (padcount == 0) {
                        return new BigInteger(data);
                    } else if (padcount == data.length) {
                        return BigInteger.valueOf(-1);
                    } else {
                        int trimmedSize = data.length - padcount;
                        byte[] trimmed = new byte[trimmedSize];
                        System.arraycopy(data, padcount, trimmed, 0, trimmedSize);
                        //System.out.println( trimmed );
                        return new BigInteger(trimmed);
                    }
                } else {
                    return new BigInteger(data);
                }
            }
            else if (signed == SIGN_MAG) {
                if ((data[0] & 0x80) > 0) {
                    byte[] data2 = (byte[])data.clone();
                    data2[0] = (byte)(data2[0] & 0x7f); // cut off highest bit
                    return new BigInteger(-1, data2);
                }
                else {
                    return new BigInteger(1, data);
                }
                
            }
            else {
                System.out.println("inaccessable code accessed!");
                return BigInteger.ONE;
                
            }
        }
        //catch (CloneNotSupportedException e) {}
        catch (IOException e) {
            //FIXME:
            e.printStackTrace();
            return BigInteger.ONE;
            
        }
        //return null; // unreachable
    }
    
    /** notification that the underlying binary has been updated */
    public void binaryUpdated() { // (BinaryUpdateEvent e) {
        //FIXME: ignore for now!
    }
    
    /** request to updated underlying binary */
    public void updateBinary(Byte[] b) { 
        //FIXME: needs to be done.
    }    
    
    /** notify binary listeners that the value (and thus the binary) has changed */
    private void alertBinaryListener(Data newdata) throws NumberFormatException {
        SelectionResource sel = getSelectionResource();
        sel.insertReplace(newdata);
    }
    
    public void setValue(BigInteger value) throws NumberFormatException {
        byte[] b = encodeBigInt(value);
        Data newdata = new ArrayData(b);
        //openFile.getEditableData().insertReplace(sel, newdata);
        alertBinaryListener(newdata);
        //System.out.println("replacing: " + sel.getMinSelectionIndex() + "-" + sel.getMaxSelectionIndex());
    }
    
    public byte[] encodeBigInt(BigInteger bigInt) {
        
        byte[] data = bigInt.toByteArray();
        SelectionData selData = getSelectionResource().getSelectionData();
        int signed = getSigned();
        
        if (data.length == 0) {
            return data;
        } else if (signed == UNSIGNED) {
            // strip leading 0x00
            if (!getAllowResize().getValue()) {
                int paddingNeeded = (int)( selData.getLength() - data.length );
                if (paddingNeeded == 0) {
                    return data;
                } else if (paddingNeeded > 0) {
                    byte[] padded = new byte[(int)selData.getLength()];
                    System.arraycopy(data, 0, padded, paddingNeeded, data.length);
                    return padded;
                } else {
                    if (paddingNeeded == -1 && data[0] == 0) {
                        byte[] strip = new byte[data.length-1];
                        System.arraycopy(data, 1, strip, 0, strip.length);
                        return strip;
                    } else {
                        System.out.println("too big!");
                        throw new IllegalArgumentException("Number too big to fit");
                        
                    }
                }
            } else {
                //fixme: assumes stretch AND shrink ok
                if (data[0] == 0) {
                    byte[] strip = new byte[data.length-1];
                    System.arraycopy(data, 1, strip, 0, strip.length);
                    return strip;
                } else {
                    return data;
                }
            }
            
        } else if (signed == ONES_COMP) {
            if (bigInt.signum() == -1) {
                return bigInt.subtract(BigInteger.ONE).toByteArray(); //FIXME: does not allow -0
            } else {
                // ones comp, positive.
                return data;
            }
        } else if (signed == TWOS_COMP) {
            // big-endian, two's comp.
            if (!getAllowResize().getValue()) {
                int paddingNeeded = (int)( selData.getLength() - data.length );
                
                if (paddingNeeded == 0) {
                    return data;
                } else if (paddingNeeded > 0) {
                    byte pad = (bigInt.signum() == -1 ? (byte)-1 : (byte)0);
                    byte[] padded = new byte[(int)selData.getLength()];
                    System.arraycopy(data, 0, padded, paddingNeeded, data.length);
                    Arrays.fill(padded, 0, paddingNeeded, pad);
                    return padded;
                } else {
                    throw new IllegalArgumentException("Number too big to fit");
                }
            } else {
                //fixme: assumes stretch AND shrink ok
                return data;
            }
        } else if (signed == SIGN_MAG) {
            //fixme: no fixed size support
            if (bigInt.signum() == -1) {
                BigInteger positive = bigInt.multiply(new BigInteger("-1")); // ugh
                data = positive.toByteArray();
                data[0] = (byte)(data[0] | 0x80 ); // set highest bit
            } else {
                return data;
            }
        } else {
            System.out.println("inaccessable code accessed!");
            return data;
        }
        
        return null; // unrearchable?
        
        //getModLayer(); //FIXME:X
        //sel.setValue(val);level4.php
        
    }
    
    public void editProperties() {
        new IntAddressedResourcePropertiesForm(this).show();
    }
    
    
}

