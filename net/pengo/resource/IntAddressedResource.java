/**
 * IntAddressedResource.java
 *
 * @author Created by Omnicore CodeGuide
 */

package net.pengo.resource;

import net.pengo.app.*;
import net.pengo.selection.*;
import net.pengo.data.*;
import net.pengo.propertyEditor.*;
import net.pengo.restree.ResourceList;

import java.util.*;
import javax.swing.*;
import javax.swing.event.*;
import java.awt.event.ActionEvent;
import java.math.*;
import java.io.IOException;


public class IntAddressedResource extends IntResource implements AddressedResource {
    
    private IntResource signedRes;
    private SelectionResource selRes;
    
    //private boolean allowStretch = false; //3:
    //private boolean allowShrink = false; //4:
    private BooleanResource allowResize = new BooleanPrimativeResource(getOpenFile(), false);

    public boolean isPrimative() {
	return false;
    }

    public IntAddressedResource(OpenFile openFile, SelectionResource selRes, IntResource signedRes) {
        super(openFile);
	
        this.selRes = selRes; // note: may be replaced as above
        this.signedRes = signedRes;
	//new IntAddressedResourcePropertiesForm(this).show();
    }
    
    public IntAddressedResource(OpenFile openFile, SelectionResource selRes, int signed) {
	this(openFile, selRes, new IntPrimativeResource(openFile, (long)signed));
    }
    
    public SelectionResource getSelectionResource() {
        return selRes;
    }
    
    public void setSelectionResource(SelectionResource selRes) {
        this.selRes = selRes;
    }
    
    public void doubleClickAction() {
	LongListSelectionModel selection = (LongListSelectionModel)selRes.getSelection().clone();
        openFile.setSelectionModel(selection);
    }

    public BigInteger getValue() {
        //byte[] data = sel.getDataStreamAsArray();
	int signed = getSigned();
	
	try
	{
		byte[] data = selRes.getSelectionData().readByteArray();
		if (data.length == 0)
		{
			return BigInteger.ZERO;
		}
		if (signed == UNSIGNED)
		{
			return new BigInteger(1, data);
		}
		else if (signed == ONES_COMP)
		{
			if ((data[0] & 0x80) > 0)
			{
				byte[] data2 = new byte[data.length];
				//int signum = -1;
				data2[0] = (byte)((~data[0]) & 0x7f); // cut off highest bit and invert rest
				for (int i=1; i<data.length; i++)
				{
					data2[i] = (byte)~data[i];
				}
				return new BigInteger(-1, data2);
			}
			else
			{
				// ones comp, positive.
				return new BigInteger(1, data);
			}
		}
		else if (signed == TWOS_COMP)
		{
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
			    System.out.println( trimmed );
			    return new BigInteger(trimmed);
			}
		    } else {
			return new BigInteger(data);
		    }
		}
		else if (signed == SIGN_MAG)
		{
			if ((data[0] & 0x80) > 0)
			{
				byte[] data2 = (byte[])data.clone();
				data2[0] = (byte)(data2[0] & 0x7f); // cut off highest bit
				return new BigInteger(-1, data2);
			}
			else
			{
				return new BigInteger(1, data);
			}
			
		}
		else
		{
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
    
    public void setValue(BigInteger value) throws NumberFormatException, IOException {
	byte[] b = encodeBigInt(value);
        Data newdata = new ArrayData(b);
        LongListSelectionModel sel = selRes.getSelection();
        openFile.getEditableData().insertReplace(sel, newdata);
        //System.out.println("replacing: " + sel.getMinSelectionIndex() + "-" + sel.getMaxSelectionIndex());
    }
    
    public byte[] encodeBigInt(BigInteger bigInt) throws IOException {

        byte[] data = bigInt.toByteArray();
        SelectionData selData = selRes.getSelectionData();
        int signed = getSigned();
	
        if (data.length == 0) {
            return data;
        } else if (signed == UNSIGNED) {
                // strip leading 0x00
            if (!allowResize.getValue()) {
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
            if (!allowResize.getValue()) {
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
    
    public void setSigned(int signage) {
        //fixme: allow to convert between?
	
	signedRes = new IntPrimativeResource(openFile, signage);
		    
        //this.signed = signage;
    }
    
    public int getSigned() {
	return signedRes.getValue().intValue();
        //return signed;
    }
    
    public void editProperties() {
	new IntAddressedResourcePropertiesForm(this).show();
    }
    
    
}

