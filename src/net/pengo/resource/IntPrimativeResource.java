/**
 * IntPrimativeResource.java
 *
 * @author Created by Omnicore CodeGuide
 */

package net.pengo.resource;

import java.math.BigInteger;

import net.pengo.propertyEditor.ResourceForm;

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
    
    public Resource[] getSources()
    {
        // primatives can have no sources, (and are defined by their value?)
        return new Resource[]{};
    }
    
    public BigInteger getValue() {
        return value;
    }
    
    public void setValue(BigInteger value) {
        value = this.value;
    }
    
}

