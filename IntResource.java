/*
 * IntResource.java
 *
 * Created on 21 August 2002, 18:09
 */

import java.util.*;
import javax.swing.*;
import javax.swing.event.*;
import java.awt.event.ActionEvent;
import java.math.*;
/**
 *
 * @author  administrator
 */
public class IntResource extends DefinitionResource {
    // final protected TransparentData sel; // from Super
    /** Creates a new instance of IntResource */
    
    final static int UNSIGNED  = 0;
    final static int ONES_COMP = 1;
    final static int TWOS_COMP = 2;
    final static int SIGN_MAG = 3;
    
    //XXX: little endian, big endian (2 byte only?)
    
    int length;
    int signed;
    Data sel;
    
    public IntResource(OpenFile openFile, Data sel, int length, int signed) {
        super(openFile);
        if (sel.getLength() != length) {
            // wrong length. set length.
            // XXX: error checking!
            sel = sel.getSelection(sel.getStart(), (long)length);
        }
        this.sel = sel; // note: may be replaced as above
        this.length = length;
        this.signed = signed;
    }
    
    public JMenu getJMenu() {
        final IntResource This = this;
        final OpenFile openFile = this.openFile;
        
  	JMenu menu = new JMenu("Example");
        Action deleteAction = new AbstractAction("Delete") {
            public void actionPerformed(ActionEvent e) {
                getOpenFile().deleteDefinition(e.getSource(), This);
            }
        };
	menu.add(deleteAction);
        
        Action untypeAction = new AbstractAction("Convert to untyped definition") {
            public void actionPerformed(ActionEvent e) {
                DefaultDefinitionResource res = new DefaultDefinitionResource(openFile, sel);
                openFile.definitionChange(e.getSource(), This, res); // xxx
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
	return menu;
    }

    public String toString() {
        return "int " + sel.toString();
    }

    public void clickAction() {
        openFile.setSelection(this, sel);
    }
    
    public BigInteger getValue() {
        byte[] data = sel.getDataStreamAsArray();
        if (data.length == 0) {
            return BigInteger.ZERO;            
        } if (signed == UNSIGNED) {
            return new BigInteger(1, data);
        } else if (signed == ONES_COMP) {
            if ((data[0] & 0x80) > 0) {
                byte[] data2 = new byte[data.length];
                int signum = -1;
                data2[0] = (byte)((~data[0]) & 0x7f); // cut off highest bit and invert rest
                for (int i=1; i<data.length; i++) {
                    data2[i] = (byte)~data[i];
                }
                return new BigInteger(-1, data2);
            } else {
                // ones comp, positive.
                return new BigInteger(1, data);
            }
        } else if (signed == TWOS_COMP) {
            // big-endian, two's comp.
            return new BigInteger(data);
        } else if (signed == SIGN_MAG) {
            if ((data[0] & 0x80) > 0) {
                byte[] data2 = (byte[])data.clone();
                data2[0] = (byte)(data2[0] & 0x7f); // cut off highest bit
                return new BigInteger(-1, data2);
            } else {
                return new BigInteger(1, data);
            }
            
        } else {
            System.out.println("inaccessable code accessed!");
            return BigInteger.ONE;
            
        }
    }
    
    public void setValue(byte[] val) {
        //getModLayer(); //xXXX
        //sel.setValue(val);
        System.out.println("value change: " + byteToInt(val));
    }
    
    public void setSigned(int signage) {
        this.signed = signage;
    }

    /* no longer used */
    public static byte[] intToByte(int i) {
        //XXX this is retarded.
        //XXX use bigInteger ?
        
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
        //XXX this is also retarded.
        
        int i = ((0xff & b[0]) << 24) | ((0xff & b[1]) << 16) | ((0xff & b[2]) << 8) | (0xff & b[3]);
        
        return i;
    }
    
}