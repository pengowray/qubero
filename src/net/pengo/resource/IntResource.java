/*
 * IntResource.java
 *
 * Created on 21 August 2002, 18:09
 */

package net.pengo.resource;

import java.awt.event.ActionEvent;
import java.io.IOException;
import java.math.BigInteger;

import javax.swing.AbstractAction;
import javax.swing.Action;
import javax.swing.JMenu;
import javax.swing.JSeparator;

import net.pengo.restree.ResourceList;

/**
 *
 * @author  administrator
 */
abstract public class IntResource extends DefinitionResource {
    /** Creates a new instance of IntResource */
    
    public final static int UNSIGNED  = 0;
    public final static int ONES_COMP = 1;
    public final static int TWOS_COMP = 2;
    public final static int SIGN_MAG = 3;
    public final static int UNUSED_SIGN_BIT = 4; //fixme: NYI: Positive signed. e.g. 0-127 only
    
    //FIXME: little endian, big endian (2 byte only?), network byte order, local byte order
    
    //FIXME: all these must be put in the subresource
    
    public IntResource() {
        super();
    }
    
    abstract public boolean isPrimative();
    
    public void giveActions(JMenu m) {
        super.giveActions(m);
        m.add(new JSeparator());
        
        Action editAction = new AbstractAction("Edit value") {
            public void actionPerformed(ActionEvent e){
                BigInteger data = IntResource.this.getValue();
                new IntInputBox("Edit value","New value:",data.toString(),IntResource.this).show();
            }
        };
        m.add(editAction);
    }
    
    /** bring up editor */
    abstract public void editProperties();
    
    public long toLong() {
        return getValue().longValue();
    }
    
    abstract public BigInteger getValue();
    
    public void setValue(String value) throws NumberFormatException, IOException {
        setValue(new BigInteger(value));
    }
    
    abstract public void setValue(BigInteger value) throws IOException;
    
    
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
    
    public String valueDesc(){
        return getValue().toString();    
    }
    
}
