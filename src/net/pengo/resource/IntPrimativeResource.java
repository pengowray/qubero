/**
 * IntPrimativeResource.java
 *
 * @author Created by Omnicore CodeGuide
 */

package net.pengo.resource;

import java.math.BigInteger;

import net.pengo.dependency.QNode;
import net.pengo.propertyEditor.IntPrimativeResourcePropertiesForm;

public class IntPrimativeResource extends IntResource
{
    
    
    private BigInteger value;
    
    public IntPrimativeResource(BigInteger value) {
        this.value = value;
    }
    
    public IntPrimativeResource(long value) {
        this(BigInteger.valueOf(value) );
    }
    
    public boolean isPrimative() {
        return true;
    }
    
    public QNode[] getSources()
    {
        // primatives can have no sources, (and are defined by their value?)
        return new QNode[]{};
    }
    
    public BigInteger getValue() {
        return value;
    }
    
    public void setValue(BigInteger value) {
        value = this.value;
    }
    
    public void editProperties() {
        new IntPrimativeResourcePropertiesForm(this).show();
    }
    
}

