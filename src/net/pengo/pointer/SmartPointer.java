/**
 * ResPointer.java
 *
 * @author Created by Omnicore CodeGuide
 */

package net.pengo.pointer;

import java.util.HashSet;

import net.pengo.resource.Resource;

public class SmartPointer extends Resource // extends QNodeResource
{
    protected Resource value;
    
    //private boolean allowNull; //xxx: implement.. also isFinal, isMutable, isVolatile, etc
    
    public SmartPointer() {
        ResourceRegistry.instance().add(this);
        
    }
//    
//    public String toString() {
//        if (getName()==null)
//            return "SmartPointer to " + value;
//        
//        return getName();
//    }
    
    public void setValue(Resource value) {
        if (this.value != null)
            this.value.removeSink(this);
        
        if (value != null)
            value.addSink(this);
        
        this.value = value;
    }
    
    /** note: use getPrimarySource for what is being pointing directly to.
     *  getValue() will "evalute" a pointer (and any pointers it points to) to get a final value 
     */
    
    public Resource evaluate() {
        //fixme: check for circular references
        HashSet circularity = new HashSet();
        circularity.add(this);
        Resource c = getPrimarySource();
        while (c.isReference()) {
            if (circularity.contains(c)) {
                //circular reference found!
                //fixme: can you spell "error handling"
                System.out.println("circular reference found! argh! abort!!");
                return null;
            }
            circularity.add(c);
            c = c.getPrimarySource();
        }
        return c;
    }
    
    public Resource getPrimarySource() {
        return value;
    }
    
    public Resource[] getSources() {
        return new Resource[]{value};
    }
    
    //fixme: is this ok?
    public boolean isAssignableFrom(Object cl) {
        return false;
    }
    
    public boolean isAssignableTo(Object cl) {
        return false;
    }
    
    public boolean isReference() {
        return true;
    }

    /* (non-Javadoc)
     * @see net.pengo.resource.QNodeResource#editProperties()
     */
    public void editProperties() {
        // TODO Auto-generated method stub
        
    }
}

