/*
 * IntResource.java
 *
 * Created on 21 August 2002, 18:09
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
/**
 *
 * @author  administrator
 */
public class IntResource extends DefinitionResource {
    // final protected TransparentData sel; // from Super
    /** Creates a new instance of IntResource */
    
    public final static int UNSIGNED  = 0;
    public final static int ONES_COMP = 1;
    public final static int TWOS_COMP = 2;
    public final static int SIGN_MAG = 3;
    public final static int UNUSED_SIGN_BIT = 4; //fixme: NYI: Positive signed. e.g. 0-127 only
    
    //FIXME: little endian, big endian (2 byte only?), network byte order, local byte order
    
    //fixme: all these must be put in the subresource
    private int signed; //0:
    //private LongListSelectionModel sel; // needed to able to set the selection //1:  use SelectionResource instead!
    //private SelectionData selData; //2:
    private boolean allowStretch = false; //3:
    private boolean allowShrink = false; //4:
    
    private IntResource signedRes; // use if not null
    private SelectionResource selRes;
    //prviate 
    
    //public IntResource(OpenFile openFile, LongListSelectionModel sel, int length, int signed) {
    public IntResource(OpenFile openFile, SelectionResource selRes, int signed) {
        super(openFile);
	/*
        rl = new ResourceList(new ArrayList(), openFile, "Sub resources") {
	    public void setSigned(int signed){add(new Integer(signed));}
	    public int getSigned(){add(new Integer(signed));}
	};
	add(new Integer(signed));
	add(sel); // 0
	 */
	
	
        this.selRes = selRes; // note: may be replaced as above
        //this.length = length;
        this.signed = signed;
	new IntResourcePropertiesForm(IntResource.this).show();
    }
    
    
    public SelectionResource getSelectionResource() {
        return selRes;
    }
    
    public JMenu getJMenu() {
        //final OpenFile openFile = this.openFile;
        
  	JMenu menu = new JMenu("Menu");
        
        Action deleteAction = new AbstractAction("Delete") {
            public void actionPerformed(ActionEvent e) {
                //getOpenFile().deleteDefinition(e.getSource(), This);
		getOpenFile().getDefinitionList().remove(IntResource.this);
            }
        };
	menu.add(deleteAction);
        
        Action untypeAction = new AbstractAction("Convert to untyped definition") {
            public void actionPerformed(ActionEvent e) {
                //DefaultDefinitionResource res = new DefaultDefinitionResource(openFile, sel);
                DefaultDefinitionResource res = new DefaultDefinitionResource(openFile, selRes.getSelection()); // change to above when DefaultDefinitionResource is fixed
		List l = getOpenFile().getDefinitionList();
		int index = l.indexOf(IntResource.this);
		l.remove(index);
		l.add(index, res);
		
            }
        };
	menu.add(untypeAction);
        
        Action editAction = new AbstractAction("Edit value") {
            public void actionPerformed(ActionEvent e){
                BigInteger data = IntResource.this.getValue();
                new IntInputBox("Edit value","New value:",data.toString(),IntResource.this).show();
            }
        };
        menu.add(editAction);

        /*
        Action unsignAction = new AbstractAction("Convert to unsigned") {
            public void actionPerformed(ActionEvent e) {
                IntResource.this.setSigned(IntResource.UNSIGNED);
                //openFile.definitionChange(e.getSource(), This, res); // xxx
            }
        };
	menu.add(unsignAction);
        
        Action onesAction = new AbstractAction("Convert to one's complement") {
            public void actionPerformed(ActionEvent e) {
                IntResource.this.setSigned(IntResource.ONES_COMP);
                //openFile.definitionChange(e.getSource(), This, res); // xxx
            }
        };
	menu.add(onesAction);

        Action twosAction = new AbstractAction("Convert to two's complement") {
            public void actionPerformed(ActionEvent e) {
                IntResource.this.setSigned(IntResource.TWOS_COMP);
                //openFile.definitionChange(e.getSource(), This, res); // xxx
            }
        };
	menu.add(twosAction);

        Action signmagAction = new AbstractAction("Convert to sign-magnitude") {
            public void actionPerformed(ActionEvent e) {
                IntResource.this.setSigned(IntResource.SIGN_MAG);
                //openFile.definitionChange(e.getSource(), This, res); // xxx
            }
        };
	menu.add(signmagAction);
        //JPopupMenu popup = menu.getPopupMenu();
        */
        
        Action propAction = new AbstractAction("Edit properties") {
            public void actionPerformed(ActionEvent e) {
                new IntResourcePropertiesForm(IntResource.this).show();
            }
        };
	menu.add(propAction);
	 
        //JPopupMenu popup = menu.getPopupMenu();
	return menu;
        
    }

    public String toString() {
        return "int " + getValue(); // sel.toString();
    }

    public void doubleClickAction() {
        openFile.setSelectionModel(selRes.getSelection());
    }
    
    public BigInteger getValue() {
        //byte[] data = sel.getDataStreamAsArray();
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
					int signum = -1;
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
    
    public void setValue(String value) throws NumberFormatException {
        byte[] b = stringToByteArray(value);
        Data newdata = new ArrayData(b);
        LongListSelectionModel sel = selRes.getSelection();
        openFile.getEditableData().insertReplace(sel, newdata);
        System.out.println("replacing: " + sel.getMinSelectionIndex() + "-" + sel.getMaxSelectionIndex());
    }
    
    public byte[] stringToByteArray(String value) {
        BigInteger bigInt = new BigInteger(value);
        byte[] data = bigInt.toByteArray();
        SelectionData selData = selRes.getSelectionData();
        
        if (data.length == 0) {
            return data;
        } else if (signed == UNSIGNED) {
                // strip leading 0x00
            if (!allowShrink && !allowStretch) {
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
            if (!allowShrink && !allowStretch) {
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
        this.signed = signage;
    }
    
    public int getSigned() {
        return signed;
    }
    
    public long toLong() {
        //fixme!! ARARGHARHH bogus error checking.
        //fixme: converts to a bloody String and back again
        try {
            return Long.parseLong( getValue().toString() );
        } catch (NumberFormatException e) {
            e.printStackTrace();
            return 0L;
        }
    }
    
    
    

    /* no longer used */
    public static byte[] intToByte(int i) {
        //FIXME: this is retarded.
        //FIXME: use bigInteger ?
        
        byte[] b = new byte[4];
        b[0] = (byte) (i >> 24);
        b[1] = (byte) (i >> 16);
        b[2] = (byte) (i >> 8);
        b[3] = (byte) (i     );
        
        return b;
        //new ObjectInputStream(new DataOutputStream(
    }
    
    /* no longer used */
    public static int byteToInt(byte[] b) {
        //FIXME: this is also retarded.
        
        int i = ((0xff & b[0]) << 24) | ((0xff & b[1]) << 16) | ((0xff & b[2]) << 8) | (0xff & b[3]);
        
        return i;
    }
    
    public ResourceList getSubResources() {
        return null;
    }
    
}
